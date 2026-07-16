use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::SubagentContext;
use crate::tool::{notification_futures, notify_any, parse_timeout, run_state_name};

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
            None => self
                .ctx
                .registry
                .all_agent_ids()
                .into_iter()
                .filter(|id| {
                    matches!(
                        self.ctx.registry.run_state(*id),
                        crate::registry::SubagentRunState::Running
                    )
                })
                .collect(),
        };
        if ids.is_empty() {
            return Ok(serde_json::json!({ "status": "ready", "results": [] }));
        }
        for id in &ids {
            if !self.ctx.registry.contains(*id) {
                return Err(ToolError::ExecutionError(format!("agent {} not found", id)));
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            let notifications =
                notification_futures(ids.iter().filter_map(|id| self.ctx.registry.notify(*id)));
            let (ready, pending): (Vec<u64>, Vec<u64>) = ids
                .iter()
                .partition(|id| self.ctx.registry.run_state(**id).is_ready());
            if pending.is_empty() {
                let results: Vec<serde_json::Value> = ready
                    .iter()
                    .map(|id| {
                        serde_json::json!({
                            "agent_id": id,
                            "run_state": run_state_name(&self.ctx.registry.run_state(*id)),
                        })
                    })
                    .collect();
                return Ok(serde_json::json!({
                    "status": "ready",
                    "results": results,
                }));
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "ready": ready,
                    "pending": pending,
                }));
            }
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifications.is_empty() {
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(notifications) => {}
                }
            }
        }
    }
}
