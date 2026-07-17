use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::SubagentContext;
use crate::registry::WaitAllOutcome;
use crate::tool::parse_timeout;

/// The `wait_all_subagents` tool: blocks until every listed subagent (or, with
/// no ids, all currently-running ones) reaches a terminal state, or until the
/// timeout elapses — then returns the ready/pending ids split. Never kills
/// running subagents on timeout.
pub struct WaitAllSubagentsTool {
    ctx: SubagentContext,
}

impl WaitAllSubagentsTool {
    /// Construct holding a clone of the shared subagent context.
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAllSubagentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_all_subagents".to_string(),
            description: "Wait until all specified subagents are ready (finished or errored), or until the timeout elapses. With no agent_ids, defaults to all of the parent's currently-running subagents. On timeout, returns the ready and pending lists separately. Does NOT kill or affect running subagents on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The agent ids to wait on. If omitted, waits on all of the parent's currently-running subagents."
                    },
                    "timeout": { "type": "number", "description": "Maximum seconds to wait. Must be positive." }
                },
                "required": ["timeout"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let timeout_secs = parse_timeout(&params)?;
        // agent_ids optional: default to all currently-running.
        let ids: Vec<u64> = match params.get("agent_ids").and_then(|v| v.as_array()) {
            Some(arr) => {
                let v: Result<Vec<u64>, ToolError> = arr
                    .iter()
                    .map(|x| {
                        x.as_u64().ok_or_else(|| {
                            ToolError::InvalidParameters("'agent_ids' must contain integers".into())
                        })
                    })
                    .collect();
                v?
            }
            None => self.ctx.registry.running_agent_ids(),
        };
        match self
            .ctx
            .registry
            .wait_all(&ids, Duration::from_secs_f64(timeout_secs))
            .await
            .map_err(|error| ToolError::ExecutionError(error.to_string()))?
        {
            WaitAllOutcome::Ready { results } => {
                let results: Vec<_> = results
                    .into_iter()
                    .map(|(agent_id, snapshot)| {
                        serde_json::json!({
                            "agent_id": agent_id,
                            "run_state": snapshot.name(),
                        })
                    })
                    .collect();
                Ok(serde_json::json!({
                    "status": "ready",
                    "results": results,
                }))
            }
            WaitAllOutcome::Timeout { ready, pending } => Ok(serde_json::json!({
                "status": "timeout",
                "ready": ready,
                "pending": pending,
            })),
        }
    }
}
