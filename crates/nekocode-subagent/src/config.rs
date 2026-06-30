use serde::{Deserialize, Serialize};

/// Per-parent middleware config for the subagent middleware. Stored as a
/// `Middleware` row's `config` JSON in the parent thread's DB row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentConfig {
    /// Maximum *nesting* depth — how many levels of subagents may spawn
    /// further subagents. The top-level thread spawns level-1 subagents
    /// (depth 0 → child at depth 1); `max_depth` bounds how deep those may
    /// nest. `max_depth = 0` (the default) means level-1 subagents cannot
    /// themselves spawn (depth 1 + 1 > 0). `max_depth = 1` allows one level
    /// of nesting. Propagated unchanged down the chain as the single
    /// tree-wide bound.
    #[serde(default)]
    pub max_depth: u32,
}

impl SubagentConfig {
    /// Deserialize from a `serde_json::Value` (the middleware row's `config`
    /// column), falling back to the default on any error.
    pub fn from_value(v: &serde_json::Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_depth_is_zero() {
        assert_eq!(SubagentConfig::default().max_depth, 0);
    }

    #[test]
    fn from_value_parses_max_depth() {
        let v = serde_json::json!({ "max_depth": 2 });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 2);
    }

    #[test]
    fn from_value_falls_back_on_invalid() {
        let v = serde_json::json!({ "max_depth": "not a number" });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }

    #[test]
    fn from_value_omits_missing_field() {
        let v = serde_json::json!({});
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }
}
