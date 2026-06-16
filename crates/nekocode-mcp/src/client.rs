use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tracing::warn;

/// A tool advertised by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

/// JSON-RPC 2.0 over stdio transport for a single MCP server process.
struct StdioTransport {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
}

impl StdioTransport {
    async fn spawn(command: &str, envs: &HashMap<String, String>) -> Result<Self> {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        cmd.envs(envs);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().context("failed to spawn MCP server")?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => {
                        warn!("MCP stdio read error: {e}");
                        break;
                    }
                }
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parsed: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
                    let mut map = pending_clone.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(parsed);
                    }
                }
            }
        });

        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            next_id: AtomicU64::new(1),
        })
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&body)?;
        line.push('\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        let resp = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .context("MCP stdio request timed out (30s)")?
            .context("MCP stdio reader dropped the request channel")?;

        if let Some(err) = resp.get("error") {
            anyhow::bail!("MCP error calling {method}: {err}");
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&body)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn kill(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

/// JSON-RPC 2.0 over Streamable HTTP transport. Each request is an independent
/// POST; the server returns the JSON-RPC response in the body (optionally via
/// SSE, but we only use the final response object).
struct HttpTransport {
    url: String,
    client: reqwest::Client,
    next_id: AtomicU64,
    /// Optional session id returned by the server via the `Mcp-Session-Id`
    /// header; sent back on subsequent requests to associate them.
    session_id: Mutex<Option<String>>,
}

impl HttpTransport {
    fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            next_id: AtomicU64::new(1),
            session_id: Mutex::new(None),
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(&body);
        {
            let sid = self.session_id.lock().await;
            if let Some(id) = sid.as_ref() {
                req = req.header("Mcp-Session-Id", id);
            }
        }

        let resp = req
            .send()
            .await
            .context("MCP HTTP request failed")?;

        // The Streamable HTTP spec lets servers return either a bare JSON
        // response or an SSE stream whose last `data:` line is the result.
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Capture the session id if the server assigned one.
        if let Some(sid) = resp.headers().get("Mcp-Session-Id").and_then(|v| v.to_str().ok()) {
            let mut guard = self.session_id.lock().await;
            *guard = Some(sid.to_string());
        }

        let result = if content_type.contains("text/event-stream") {
            extract_last_event_data(resp).await?
        } else {
            resp.json::<Value>().await.context("MCP HTTP: bad JSON body")?
        };

        if let Some(err) = result.get("error") {
            anyhow::bail!("MCP HTTP error calling {method}: {err}");
        }
        Ok(result.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(&body);
        {
            let sid = self.session_id.lock().await;
            if let Some(id) = sid.as_ref() {
                req = req.header("Mcp-Session-Id", id);
            }
        }
        // Notifications don't expect a response body; ignore it.
        let _ = req.send().await?;
        Ok(())
    }
}

/// Parse an SSE stream and return the JSON value from the last `data:` event.
/// The Streamable HTTP spec delivers the JSON-RPC response as the payload of
/// the terminal SSE message.
async fn extract_last_event_data(resp: reqwest::Response) -> Result<Value> {
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        if let Ok(bytes) = chunk {
            buf.extend_from_slice(&bytes);
        }
    }
    let text = String::from_utf8_lossy(&buf);
    // An SSE event is a block of `data: <line>` lines separated by blank
    // lines. Find the last such block and parse its concatenated payload.
    let mut last_data: Option<String> = None;
    for block in text.split("\n\n") {
        let data: String = block
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(|s| s.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            last_data = Some(data);
        }
    }
    let data = last_data.context("MCP HTTP SSE stream had no data event")?;
    Ok(serde_json::from_str(&data).context("MCP HTTP SSE: bad JSON in data event")?)
}

/// Unified MCP client abstracting over stdio and Streamable HTTP transports.
pub struct McpClient {
    transport: Transport,
}

enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

impl McpClient {
    /// Spawn an MCP server over stdio.
    pub async fn spawn(command: &str, envs: &HashMap<String, String>) -> Result<Self> {
        let t = StdioTransport::spawn(command, envs).await?;
        Ok(Self {
            transport: Transport::Stdio(t),
        })
    }

    /// Connect to an MCP server over Streamable HTTP.
    pub fn connect_http(url: String) -> Self {
        Self {
            transport: Transport::Http(HttpTransport::new(url)),
        }
    }

    /// Send a JSON-RPC request and return the `result` field.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        match &self.transport {
            Transport::Stdio(t) => t.request(method, params).await,
            Transport::Http(t) => t.request(method, params).await,
        }
    }

    /// Perform the MCP `initialize` handshake.
    pub async fn initialize(&self) -> Result<()> {
        let _ = self
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "nekocode", "version": "0.1.0" },
                }),
            )
            .await?;
        self.notify("notifications/initialized", Value::Null).await?;
        Ok(())
    }

    /// Send a JSON-RPC notification (no response expected).
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio(t) => t.notify(method, params).await,
            Transport::Http(t) => t.notify(method, params).await,
        }
    }

    /// Call `tools/list`.
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        let result = self.request("tools/list", serde_json::json!({})).await?;
        let tools_val = result
            .get("tools")
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        let tools: Vec<McpToolInfo> = serde_json::from_value(tools_val)
            .context("failed to parse MCP tools/list response")?;
        Ok(tools)
    }

    /// Call `tools/call`.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({ "name": name, "arguments": args }),
            )
            .await?;
        Ok(result)
    }

    /// Kill the server process (stdio mode). No-op for HTTP.
    pub async fn kill(&self) {
        if let Transport::Stdio(t) = &self.transport {
            t.kill().await;
        }
    }
}