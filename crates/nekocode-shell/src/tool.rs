use std::process::Stdio;

use nekocode_types::tool::Tool;

pub struct ShellTool {}

#[async_trait::async_trait]
impl Tool for ShellTool {
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
            }))
        } else {
            Err(nekocode_types::tool::ToolError::ExecutionError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}
