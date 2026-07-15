use serde::{Deserialize, Serialize};

/// Per-thread configuration for the skills middleware.
///
/// Stored as the `config` JSON column on the `Middleware` entity row.
/// Example JSON: `{ "enabled": ["skill-creator", "tdd"] }`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillsConfig {
    /// Names of skills whose catalog entries are injected into the system
    /// prompt. Full instructions are loaded on demand through `read_skill`.
    #[serde(default)]
    pub enabled: Vec<String>,
}

impl SkillsConfig {
    /// Best-effort deserialization: a missing or malformed config falls back to
    /// defaults rather than failing to activate the thread.
    pub fn from_value(v: &serde_json::Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    /// Serialize back to the JSON form persisted on the `Middleware` entity
    /// row's `config` column (camelCase keys, matching `from_value`).
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_falls_back_to_default() {
        let cfg = SkillsConfig::from_value(&serde_json::Value::Null);
        assert!(cfg.enabled.is_empty());
    }

    #[test]
    fn deserializes_enabled_list() {
        let v = serde_json::json!({ "enabled": ["skill-creator", "tdd"] });
        let cfg = SkillsConfig::from_value(&v);
        assert_eq!(cfg.enabled, vec!["skill-creator", "tdd"]);
    }

    #[test]
    fn roundtrip() {
        let cfg = SkillsConfig {
            enabled: vec!["a".into(), "b".into()],
        };
        let v = cfg.to_value();
        let back = SkillsConfig::from_value(&v);
        assert_eq!(back.enabled, vec!["a", "b"]);
    }
}
