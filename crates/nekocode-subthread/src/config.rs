use serde::{Deserialize, Serialize};

/// Per-thread configuration for the subthread middleware. Stored as the
/// `config` JSON column on the `Middleware` entity row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubthreadConfig {
    /// Whether subthreads spawned from this thread may themselves spawn
    /// sub-subthreads. Default `false` to bound recursion depth.
    ///
    /// `skip_serializing_if = "is_false"` keeps the default round-tripping
    /// as `{}` rather than `{"allowSubthread": false}`, matching the empty
    /// shape of other middleware configs (`ShellConfig`, `FileConfig`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_subthread: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl SubthreadConfig {
    /// Best-effort deserialization: a missing or malformed config falls back
    /// to defaults rather than failing to activate the thread. Mirrors the
    /// pattern in `nekocode_shell::config::ShellConfig::from_value`.
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
    fn null_falls_back_to_default() {
        let cfg = SubthreadConfig::from_value(&serde_json::Value::Null);
        assert!(!cfg.allow_subthread);
    }

    #[test]
    fn deserializes_allow_subthread() {
        let v = serde_json::json!({ "allowSubthread": true });
        let cfg = SubthreadConfig::from_value(&v);
        assert!(cfg.allow_subthread);
    }

    #[test]
    fn roundtrip() {
        let cfg = SubthreadConfig { allow_subthread: true };
        let v = cfg.to_value();
        let back = SubthreadConfig::from_value(&v);
        assert!(back.allow_subthread);
    }

    #[test]
    fn default_is_empty_object() {
        // Default must serialize to `{}` so the JSON column round-trips.
        let v = SubthreadConfig::default().to_value();
        assert_eq!(v, serde_json::json!({}));
    }
}
