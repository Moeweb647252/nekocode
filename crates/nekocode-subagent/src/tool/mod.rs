use nekocode_types::tool::ToolError;

pub mod abort_subagent;
pub mod inspect_subagent;
pub mod read_subagent;
pub mod spawn_subagent;
pub mod wait_all_subagents;
pub mod wait_any_subagent;

pub use abort_subagent::AbortSubagentTool;
pub use inspect_subagent::InspectSubagentTool;
pub use read_subagent::ReadSubagentTool;
pub use spawn_subagent::SpawnSubagentTool;
pub use wait_all_subagents::WaitAllSubagentsTool;
pub use wait_any_subagent::WaitAnySubagentTool;

/// Parse a single `agent_id` (u64) parameter.
pub(crate) fn parse_agent_id(params: &serde_json::Value) -> Result<u64, ToolError> {
    params
        .get("agent_id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing or invalid 'agent_id' parameter".into())
        })
}

/// Parse a non-empty `agent_ids` array parameter.
pub(crate) fn parse_agent_ids(params: &serde_json::Value) -> Result<Vec<u64>, ToolError> {
    let arr = params
        .get("agent_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing 'agent_ids' array parameter".into())
        })?;
    if arr.is_empty() {
        return Err(ToolError::InvalidParameters(
            "'agent_ids' must be a non-empty array".into(),
        ));
    }
    arr.iter()
        .map(|v| {
            v.as_u64().ok_or_else(|| {
                ToolError::InvalidParameters("'agent_ids' must contain integers".into())
            })
        })
        .collect()
}

/// Parse a positive `timeout` (seconds, f64) parameter.
pub(crate) fn parse_timeout(params: &serde_json::Value) -> Result<f64, ToolError> {
    let t = params
        .get("timeout")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing or invalid 'timeout' parameter".into())
        })?;
    if !t.is_finite() || t <= 0.0 {
        return Err(ToolError::InvalidParameters(
            "'timeout' must be positive and finite".into(),
        ));
    }
    Ok(t)
}
