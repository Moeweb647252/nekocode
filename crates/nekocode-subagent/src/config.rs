use serde::{Deserialize, Serialize};

/// Configuration for the `nekocode-subagent` middleware.
///
/// Currently minimal — only an `allow_subagent` flag. The per-subagent
/// Provider is inherited from the parent agent at construction time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    /// Whether subagents spawned by this middleware may themselves spawn
    /// sub-subagents. Default `false` — each parent must opt its children
    /// into further nesting.
    #[serde(default)]
    pub allow_subagent: bool,
}

impl SubagentConfig {
    pub fn from_value(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}
