use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{self, AtomicU32, AtomicUsize},
    },
    time::Duration,
};

use nekocode_types::tool::{Tool, ToolError};
use sdd::{AtomicOwned, Guard};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};
use tracing::debug;

use crate::{ShellTaskState, config::ShellConfig};

/// Push a line into the shared output buffer. The buffer is wrapped in an
/// `AtomicOwned` so it can be swapped atomically; readers must load under a
/// [`sdd::Guard`].
fn push_output(buf: &AtomicOwned<boxcar::Vec<String>>, line: String) {
    let guard = Guard::new();
    if let Some(v) = buf.load(atomic::Ordering::Acquire, &guard).as_ref() {
        v.push(line);
    }
}

/// One-shot shell execution: spawn `<shell> -c <command>`, capture stdout /
/// stderr, and return them along with the exit code. Honors the working
/// directory, env vars, and timeout from [`ShellConfig`].
pub struct OnceShellTool {
    pub config: Arc<ShellConfig>,
}

#[async_trait::async_trait]
impl Tool for OnceShellTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "shell".to_string(),
            description:
                "A tool for executing a one-shot shell command. The cwd is working directory."
                    .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute."
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'command' parameter".into()))?;

        let mut cmd = tokio::process::Command::new(self.config.program());
        cmd.arg("-c").arg(command);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.config.apply(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

        // Take the pipes so we can wait + kill via the (borrowing) `wait`,
        // while still collecting stdout/stderr.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::ExecutionError("Failed to capture stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::ExecutionError("Failed to capture stderr".into()))?;

        let collect = async {
            use tokio::io::AsyncReadExt;
            let (mut stdout, mut stderr) = (stdout, stderr);
            let (mut out_buf, mut err_buf) = (Vec::new(), Vec::new());
            let (out, err) = tokio::join!(
                stdout.read_to_end(&mut out_buf),
                stderr.read_to_end(&mut err_buf),
            );
            out?;
            err?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((out_buf, err_buf, status))
        };

        let (stdout, stderr, status) = if let Some(secs) = self.config.timeout_secs {
            match tokio::time::timeout(Duration::from_secs(secs), collect).await {
                Ok(res) => res.map_err(|e| ToolError::ExecutionError(e.to_string()))?,
                Err(_) => {
                    // Timed out. Kill the process and reap it so it doesn't
                    // become a zombie; SIGTERM may be ignored by a trapped
                    // child, so escalate to SIGKILL if it hasn't exited.
                    let _ = child.start_kill();
                    let killed = tokio::time::timeout(Duration::from_secs(2), child.wait())
                        .await
                        .is_ok();
                    if !killed {
                        // Forcefully kill and reap to avoid leaking a zombie.
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                    }
                    return Err(ToolError::ExecutionError(format!(
                        "command timed out after {secs}s"
                    )));
                }
            }
        } else {
            collect
                .await
                .map_err(|e| ToolError::ExecutionError(e.to_string()))?
        };

        // Always return a structured result so the model can see stdout even
        // on non-zero exits (previously the whole result was lost into an
        // error string).
        Ok(serde_json::json!({
            "stdout": String::from_utf8_lossy(&stdout),
            "stderr": String::from_utf8_lossy(&stderr),
            "exit_code": status.code(),
        }))
    }
}

/// Spawn a long-running shell process. Output is buffered into an append-only
/// ring per shell id and read incrementally via `fetch_shell_output`. A
/// single supervisor task owns the child, drains stdout + stderr to EOF
/// independently, and cleans up the shell state on exit or cancellation.
pub struct SpawnShellTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
    pub config: Arc<ShellConfig>,
    pub allocate_id: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl Tool for SpawnShellTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "spawn_shell".to_string(),
            description:
                "A tool for spawning a long-running shell process. The cwd is working directory."
                    .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute."
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'command' parameter".into()))?;

        let mut cmd = tokio::process::Command::new(self.config.program());
        cmd.arg("-c").arg(command);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.config.apply(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;
        let pid = child.id();

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::ExecutionError("Failed to capture stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::ExecutionError("Failed to capture stderr".into()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::ExecutionError("Failed to capture stdin".into()))?;

        let shell_id = self.allocate_id.fetch_add(1, atomic::Ordering::Relaxed);
        let output = Arc::new(AtomicOwned::new(boxcar::Vec::new()));
        let output_cursor = Arc::new(AtomicUsize::new(0));
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();
        let is_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let cancellation_token = tokio_util::sync::CancellationToken::new();

        self.shell_states.insert(
            shell_id,
            ShellTaskState {
                shell_id,
                pid,
                command: command.to_string(),
                output: output.clone(),
                output_cursor: output_cursor.clone(),
                input: input_tx,
                cancellation_token: cancellation_token.clone(),
                is_running: is_running.clone(),
            },
        );

        let shell_states = self.shell_states.clone();

        tokio::spawn(async move {
            // Reader tasks: each runs its stream to EOF independently, so a
            // stdout EOF no longer drops the trailing stderr lines.
            let out = output.clone();
            let stdout_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => push_output(&out, line),
                        Ok(None) => break,
                        Err(e) => {
                            debug!("Error reading stdout: {}", e);
                            break;
                        }
                    }
                }
            });
            let out = output.clone();
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => push_output(&out, line),
                        Ok(None) => break,
                        Err(e) => {
                            debug!("Error reading stderr: {}", e);
                            break;
                        }
                    }
                }
            });

            // Stdin pump: forward user input to the child. Terminating the
            // channel (all SendShellInputTool clones dropped) closes stdin.
            let stdin_task = tokio::spawn(async move {
                while let Some(input) = input_rx.recv().await {
                    if let Err(e) = stdin.write_all(input.as_bytes()).await {
                        debug!("Error writing to stdin: {}", e);
                        break;
                    }
                    // Only append a newline when the caller didn't already
                    // terminate the line, so interactive typing is possible.
                    if !input.ends_with('\n')
                        && let Err(e) = stdin.write_all(b"\n").await
                    {
                        debug!("Error writing newline to stdin: {}", e);
                        break;
                    }
                    let _ = stdin.flush().await;
                }
            });

            // Wait for either cancellation or natural exit, then reap.
            let exit_status = tokio::select! {
                _ = cancellation_token.cancelled() => {
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
                status = child.wait() => status.ok(),
            };

            // Drain remaining output and stop pumps.
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            // Closing stdin makes the pump exit cleanly.
            drop(stdin_task);

            is_running.store(false, atomic::Ordering::SeqCst);
            if let Some(status) = exit_status {
                push_output(
                    &output,
                    format!("[exit_code={}]", status.code().unwrap_or(-1)),
                );
            } else {
                push_output(&output, "[terminated]".to_string());
            }
            shell_states.remove(&shell_id);
        });

        Ok(serde_json::json!({
            "shell_id": shell_id,
            "pid": pid,
        }))
    }
}

/// Cancel a previously spawned long-running shell.
pub struct CancelShellTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for CancelShellTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "cancel_shell".to_string(),
            description: "A tool for cancelling a long-running shell process.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "shell_id": {
                        "type": "integer",
                        "description": "The shell id returned by spawn_shell."
                    }
                },
                "required": ["shell_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let shell_id = parse_shell_id(&params)?;
        match self.shell_states.get(&shell_id) {
            Some(entry) => {
                entry.cancellation_token.cancel();
                Ok(serde_json::json!({ "status": "cancelled", "shell_id": shell_id }))
            }
            None => Err(ToolError::InvalidParameters(format!(
                "No active shell with shell_id {}",
                shell_id
            ))),
        }
    }
}

/// Send input (a line) to the stdin of a previously spawned shell.
pub struct SendShellInputTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for SendShellInputTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "send_shell_input".to_string(),
            description: "A tool for sending input to a long-running shell process.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "shell_id": {
                        "type": "integer",
                        "description": "The shell id returned by spawn_shell."
                    },
                    "input": {
                        "type": "string",
                        "description": "The input to send to the shell. A newline is appended automatically unless the input already ends with one."
                    }
                },
                "required": ["shell_id", "input"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let shell_id = parse_shell_id(&params)?;
        let input = params
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'input' parameter".into()))?;

        match self.shell_states.get(&shell_id) {
            Some(entry) => {
                if !entry.is_running.load(atomic::Ordering::SeqCst) {
                    return Err(ToolError::ExecutionError(
                        "Shell process is not running".into(),
                    ));
                }
                if let Err(e) = entry.input.send(input.to_string()) {
                    debug!("Error sending input to shell: {}", e);
                    return Err(ToolError::ExecutionError(
                        "Failed to send input to shell".into(),
                    ));
                }
                Ok(serde_json::json!({ "status": "input sent", "shell_id": shell_id }))
            }
            None => Err(ToolError::InvalidParameters(format!(
                "No active shell with shell_id {}",
                shell_id
            ))),
        }
    }
}

/// Fetch output produced since the previous fetch for a shell. Returns the new
/// lines joined by `\n`, plus the running flag. Reads are incremental and
/// lossless: the cursor is advanced after each fetch.
pub struct FetchShellOutputTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for FetchShellOutputTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "fetch_shell_output".to_string(),
            description:
                "A tool for fetching the output produced by a long-running shell since the last fetch."
                    .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "shell_id": {
                        "type": "integer",
                        "description": "The shell id returned by spawn_shell."
                    }
                },
                "required": ["shell_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let shell_id = parse_shell_id(&params)?;
        let entry = self
            .shell_states
            .get(&shell_id)
            .ok_or_else(|| {
                ToolError::InvalidParameters(format!("No active shell with shell_id {}", shell_id))
            })?
            .clone();

        let new_output = drain_output(&entry);
        Ok(serde_json::json!({
            "shell_id": shell_id,
            "output": new_output,
            "is_running": entry.is_running.load(atomic::Ordering::SeqCst),
        }))
    }
}

/// Wait for a previously spawned long-running shell to finish, blocking up to
/// a caller-supplied timeout. On completion, returns the (possibly empty) tail
/// of output produced since the last fetch plus the exit status. On timeout the
/// call still succeeds — it simply tells the model the shell is still running,
/// so the model can decide whether to wait again or cancel. The shell is never
/// killed by this tool.
pub struct WaitShellDoneTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for WaitShellDoneTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "wait_shell_done".to_string(),
            description: "Block until a previously spawned shell finishes (process exits) or the timeout elapses. Use after spawning a command whose completion you need before proceeding; this avoids busy-polling with fetch_shell_output. On timeout the call returns a non-error 'timeout' status describing the still-running shell — it does NOT kill the process."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "shell_id": {
                        "type": "integer",
                        "description": "The shell id returned by spawn_shell."
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Maximum time to wait, in seconds. Must be positive. If the shell does not finish within this time, the call returns a 'timeout' status (the shell keeps running)."
                    }
                },
                "required": ["shell_id", "timeout"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let shell_id = parse_shell_id(&params)?;
        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'timeout' parameter".into()))?;
        if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
            return Err(ToolError::InvalidParameters(format!(
                "'timeout' must be a positive number of seconds, got {timeout_secs}"
            )));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);

        loop {
            // The shell is "done" when either its is_running flag flips to
            // false (supervisor sets this right after the process exits) or the
            // entry is removed from the map (supervisor removes it immediately
            // after, so an absent entry also means done).
            let done_state = match self.shell_states.get(&shell_id) {
                Some(entry) => {
                    if !entry.is_running.load(atomic::Ordering::SeqCst) {
                        // Finished but the entry may still be present for a
                        // brief window; drain the tail so the caller sees the
                        // final output and the [exit_code=...] marker.
                        Some(drain_output(&entry))
                    } else {
                        None
                    }
                }
                None => {
                    // Entry gone: the shell already finished and was cleaned
                    // up before we observed it. There is no tail to return.
                    Some(String::new())
                }
            };
            if let Some(tail) = done_state {
                return Ok(serde_json::json!({
                    "shell_id": shell_id,
                    "status": "done",
                    "output": tail,
                }));
            }

            // Check the timeout before sleeping; if it has elapsed, tell the
            // model the shell is still running (non-error) and return.
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "shell_id": shell_id,
                    "status": "timeout",
                    "message": "The shell is still running; the wait timed out. The process was not killed. You may call wait_shell_done again, fetch_shell_output, or cancel_shell."
                }));
            }

            // Sleep until the earlier of the deadline and the next poll tick.
            let poll = tokio::time::Instant::now()
                .checked_add(Duration::from_millis(100))
                .unwrap_or(deadline);
            tokio::time::sleep_until(poll.min(deadline)).await;
        }
    }
}

/// Read all lines at index >= cursor from the buffer, then advance the cursor
/// to the new length. Returns the joined new lines.
fn drain_output(state: &ShellTaskState) -> String {
    let guard = Guard::new();
    let Some(buf) = state
        .output
        .load(atomic::Ordering::Acquire, &guard)
        .as_ref()
    else {
        // Buffer swapped out concurrently; nothing to read this round.
        return String::new();
    };
    let start = state.output_cursor.load(atomic::Ordering::Acquire);
    let total = buf.count();
    if start >= total {
        return String::new();
    }
    let mut collected: Vec<String> = Vec::with_capacity(total - start);
    for i in start..total {
        if let Some(line) = buf.get(i) {
            collected.push(line.clone());
        }
    }
    state.output_cursor.store(total, atomic::Ordering::Release);
    collected.join("\n")
}

fn parse_shell_id(params: &serde_json::Value) -> Result<u32, ToolError> {
    params
        .get("shell_id")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing or invalid 'shell_id' parameter".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use serde_json::json;

    /// Helper to build a default OnceShellTool.
    fn once_tool() -> OnceShellTool {
        OnceShellTool { config: Arc::new(ShellConfig::default()) }
    }

    #[tokio::test]
    async fn echo_returns_stdout() {
        let tool = once_tool();
        let result = tool.call(json!({"command": "echo hello world"})).await.unwrap();
        assert_eq!(result["exit_code"], 0);
        let out = result["stdout"].as_str().unwrap_or("");
        assert!(out.trim().contains("hello world"), "stdout: {out:?}");
    }

    #[tokio::test]
    async fn true_exit_is_ok() {
        let tool = once_tool();
        let result = tool.call(json!({"command": "true"})).await.unwrap();
        assert_eq!(result["exit_code"], 0);
    }

    #[tokio::test]
    async fn false_exit_is_nonzero() {
        let tool = once_tool();
        let result = tool.call(json!({"command": "false"})).await.unwrap();
        assert_eq!(result["exit_code"], 1);
    }

    #[tokio::test]
    async fn stderr_is_captured() {
        let tool = once_tool();
        let result = tool.call(json!({"command": "echo err >&2"})).await.unwrap();
        let stderr = result["stderr"].as_str().unwrap_or("");
        assert!(!stderr.trim().is_empty(), "expected non-empty stderr");
    }

    #[tokio::test]
    async fn missing_command_returns_error() {
        let tool = once_tool();
        let result = tool.call(json!({})).await;
        assert!(result.is_err(), "expected InvalidParameters error");
    }

    #[tokio::test]
    async fn spec_returns_expected_name() {
        let tool = once_tool();
        let spec = tool.spec();
        assert_eq!(spec.name, "shell");
        assert!(spec.parameter_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("command")));
    }

    #[tokio::test]
    async fn working_directory_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();
        let tool = OnceShellTool {
            config: Arc::new(ShellConfig {
                working_directory: Some(dir_path.to_string_lossy().to_string()),
                ..Default::default()
            }),
        };
        let result = tool.call(json!({"command": "pwd"})).await.unwrap();
        let out = result["stdout"].as_str().unwrap_or("");
        assert!(
            out.trim() == dir_path.to_string_lossy(),
            "expected cwd={:?}, got stdout={out:?}",
            dir_path,
        );
    }

    #[tokio::test]
    async fn env_vars_are_applied() {
        use std::collections::HashMap;
        let tool = OnceShellTool {
            config: Arc::new(ShellConfig {
                envs: HashMap::from([("MY_VAR".into(), "hello_world".into())]),
                ..Default::default()
            }),
        };
        let result = tool.call(json!({"command": "echo $MY_VAR"})).await.unwrap();
        let out = result["stdout"].as_str().unwrap_or("");
        assert!(out.trim().contains("hello_world"), "stdout: {out:?}");
    }

    #[tokio::test]
    async fn timeout_kills_command() {
        let tool = OnceShellTool {
            config: Arc::new(ShellConfig {
                timeout_secs: Some(1),
                ..Default::default()
            }),
        };
        let result = tool.call(json!({"command": "sleep 10"})).await;
        assert!(result.is_err(), "expected timeout error");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("timed out"),
            "expected timeout, got: {err:?}",
        );
    }
}
