use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{self, AtomicBool},
    },
};

use nekocode_types::tool::Tool;
use sdd::{AtomicOwned, Guard, Owned, Tag};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    select,
    sync::mpsc,
};
use tracing::debug;

use crate::ShellTaskState;

pub struct OnceShellTool {}

#[async_trait::async_trait]
impl Tool for OnceShellTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "shell".to_string(),
            description: "A tool for executing shell commands.".to_string(),
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

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                nekocode_types::tool::ToolError::InvalidParameters(
                    "Missing 'command' parameter".into(),
                )
            })?;

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .await
            .map_err(|e| nekocode_types::tool::ToolError::ExecutionError(e.to_string()))?;

        if output.status.success() {
            Ok(serde_json::json!({
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "exit_code": output.status.code(),
            }))
        } else {
            Err(nekocode_types::tool::ToolError::ExecutionError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}

pub struct SpawnShellTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for SpawnShellTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "spawn_shell".to_string(),
            description: "A tool for spawning a long-running shell process.".to_string(),
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

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                nekocode_types::tool::ToolError::InvalidParameters(
                    "Missing 'command' parameter".into(),
                )
            })?;

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| nekocode_types::tool::ToolError::ExecutionError(e.to_string()))?;

        let pid = child.id().ok_or_else(|| {
            nekocode_types::tool::ToolError::ExecutionError("Failed to get child process ID".into())
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            nekocode_types::tool::ToolError::ExecutionError("Failed to capture stdout".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            nekocode_types::tool::ToolError::ExecutionError("Failed to capture stderr".into())
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            nekocode_types::tool::ToolError::ExecutionError("Failed to capture stdin".into())
        })?;
        let output = Arc::new(AtomicOwned::new(boxcar::Vec::new()));
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        let is_running = Arc::new(AtomicBool::new(true));
        let cancellation_token = tokio_util::sync::CancellationToken::new();
        self.shell_states.insert(
            pid,
            ShellTaskState {
                output: output.clone(),
                input: input_tx,
                cancellation_token: cancellation_token.clone(),
                is_running: is_running.clone(),
            },
        );

        tokio::spawn(async move {
            let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
            let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let guard = Guard::new();
                                if let Some(output) = output.load(atomic::Ordering::SeqCst, &guard).as_ref() {
                                    output.push(line);
                                }
                            }
                            Ok(None) => break, // EOF
                            Err(e) => {
                                debug!("Error reading stdout: {}", e);
                                break;
                            }
                        }
                    }
                    line = stderr_reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let guard = Guard::new();
                                if let Some(output) = output.load(atomic::Ordering::SeqCst, &guard).as_ref() {
                                    output.push(line);
                                }
                            }
                            Ok(None) => break, // EOF
                            Err(e) => {
                                debug!("Error reading stderr: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(input) = input_rx.recv().await {
                if let Err(e) = stdin.write_all(input.as_bytes()).await {
                    debug!("Error writing to stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    debug!("Error writing newline to stdin: {}", e);
                    break;
                }
            }
        });

        let shell_states = self.shell_states.clone();
        tokio::spawn(async move {
            select! {
                _ = cancellation_token.cancelled() => {
                    let _ = child.kill().await;
                    is_running.store(false, atomic::Ordering::SeqCst);
                    shell_states.remove(&pid);
                }
                _ = child.wait() => {
                    is_running.store(false, atomic::Ordering::SeqCst);
                }
            }
        });

        Ok(serde_json::json!({
            "pid": pid,
        }))
    }
}

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
                    "pid": {
                        "type": "integer",
                        "description": "The process ID of the shell to cancel."
                    }
                },
                "required": ["pid"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        let pid = params.get("pid").and_then(|v| v.as_u64()).ok_or_else(|| {
            nekocode_types::tool::ToolError::InvalidParameters(
                "Missing or invalid 'pid' parameter".into(),
            )
        })? as u32;

        if let Some(entry) = self.shell_states.get(&pid) {
            entry.cancellation_token.cancel();
            Ok(serde_json::json!({
                "status": "cancelled"
            }))
        } else {
            Err(nekocode_types::tool::ToolError::InvalidParameters(format!(
                "No active shell with pid {}",
                pid
            )))
        }
    }
}

pub struct FetchShellOutputTool {
    pub shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>,
}

#[async_trait::async_trait]
impl Tool for FetchShellOutputTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "fetch_shell_output".to_string(),
            description: "A tool for fetching the output of a long-running shell process."
                .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "integer",
                        "description": "The process ID of the shell to fetch output from."
                    }
                },
                "required": ["pid"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        let pid = params.get("pid").and_then(|v| v.as_u64()).ok_or_else(|| {
            nekocode_types::tool::ToolError::InvalidParameters(
                "Missing or invalid 'pid' parameter".into(),
            )
        })? as u32;

        if let Some(entry) = self.shell_states.get(&pid) {
            let output = entry.output.swap(
                (Some(Owned::new(boxcar::Vec::new())), Tag::None),
                atomic::Ordering::SeqCst,
            );
            let output_string: String = output
                .0
                .map(|v| {
                    let mut sorted = v.iter().collect::<Vec<(usize, &String)>>();
                    sorted.sort_by(|a, b| a.0.cmp(&b.0));
                    sorted
                        .into_iter()
                        .map(|v| v.1.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or(String::new());
            Ok(serde_json::json!({
                "output": output_string,
                "is_running": entry.is_running.load(atomic::Ordering::SeqCst),
            }))
        } else {
            Err(nekocode_types::tool::ToolError::InvalidParameters(format!(
                "No active shell with pid {}",
                pid
            )))
        }
    }
}

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
                    "pid": {
                        "type": "integer",
                        "description": "The process ID of the shell to send input to."
                    },
                    "input": {
                        "type": "string",
                        "description": "The input to send to the shell. Ends with a newline if you want to flush the stdin."
                    }
                },
                "required": ["pid", "input"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        let pid = params.get("pid").and_then(|v| v.as_u64()).ok_or_else(|| {
            nekocode_types::tool::ToolError::InvalidParameters(
                "Missing or invalid 'pid' parameter".into(),
            )
        })? as u32;
        let input = params
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                nekocode_types::tool::ToolError::InvalidParameters(
                    "Missing 'input' parameter".into(),
                )
            })?;

        if let Some(entry) = self.shell_states.get(&pid) {
            if entry.is_running.load(atomic::Ordering::SeqCst) {
                if let Err(e) = entry.input.send(input.to_string()) {
                    debug!("Error sending input to shell: {}", e);
                    return Err(nekocode_types::tool::ToolError::ExecutionError(
                        "Failed to send input to shell".into(),
                    ));
                }
                Ok(serde_json::json!({
                    "status": "input sent"
                }))
            } else {
                Err(nekocode_types::tool::ToolError::ExecutionError(
                    "Shell process is not running".into(),
                ))
            }
        } else {
            Err(nekocode_types::tool::ToolError::InvalidParameters(format!(
                "No active shell with pid {}",
                pid
            )))
        }
    }
}
