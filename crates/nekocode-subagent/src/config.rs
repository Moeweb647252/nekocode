use serde::{Deserialize, Serialize};

/// Per-parent middleware config for the subagent middleware. Stored as a
/// `Middleware` row's `config` JSON in the parent thread's DB row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubagentConfig {
    /// Maximum *nesting* depth — how many levels of subagents may spawn
    /// further subagents. The top-level thread spawns level-1 subagents
    /// (depth 0 → child at depth 1); `max_depth` bounds how deep those may
    /// nest. `max_depth = 0` (the default) means level-1 subagents cannot
    /// themselves spawn (depth 1 + 1 > 0). `max_depth = 1` allows one level
    /// of nesting. Propagated unchanged down the chain as the single
    /// tree-wide bound.
    pub max_depth: u32,
}

impl SubagentConfig {
    /// Deserialize from a `serde_json::Value` (the middleware row's `config`
    /// column), falling back to the default on any error.
    pub fn from_value(v: &serde_json::Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    /// Best-effort serialization mirroring `from_value`.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
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
        let v = serde_json::json!({ "maxDepth": 2 });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 2);
    }

    #[test]
    fn from_value_falls_back_on_invalid() {
        let v = serde_json::json!({ "maxDepth": "not a number" });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }

    #[test]
    fn from_value_omits_missing_field() {
        let v = serde_json::json!({});
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }

    #[test]
    fn null_falls_back_to_default() {
        assert_eq!(SubagentConfig::from_value(&serde_json::Value::Null).max_depth, 0);
    }

    #[test]
    fn to_value_roundtrips_max_depth() {
        let cfg = SubagentConfig { max_depth: 3 };
        let v = cfg.to_value();
        let back = SubagentConfig::from_value(&v);
        assert_eq!(back.max_depth, 3);
    }
}
