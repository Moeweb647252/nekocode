use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Typed configuration for the `tool` middleware. Deserialized from the
/// per-thread `Middleware.config` JSON column (`{}` by default). Mirrors the
/// shape of [`nekocode_shell::config::ShellConfig`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileConfig {
    /// Working directory used to resolve relative paths. When `None`, relative
    /// paths resolve against the server's current directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

impl FileConfig {
    /// Best-effort deserialization: a malformed config falls back to defaults
    /// rather than failing to activate the thread.
    pub fn from_value(v: &serde_json::Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    /// Best-effort serialization mirroring [`Self::from_value`]. A failure
    /// falls back to `null` so the caller can persist *something* rather than
    /// rejecting the write.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Resolve a caller-supplied path against the working directory. Absolute
    /// paths are returned unchanged; relative paths are joined onto
    /// `working_directory` when it is set.
    pub fn resolve_path(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        match self.working_directory.as_deref() {
            Some(base) => Path::new(base).join(path),
            None => path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_is_unchanged() {
        let cfg = FileConfig {
            working_directory: Some("/srv/app".into()),
        };
        assert_eq!(cfg.resolve_path("/etc/hosts"), PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn relative_path_joined_onto_working_directory() {
        let cfg = FileConfig {
            working_directory: Some("/srv/app".into()),
        };
        assert_eq!(cfg.resolve_path("src/main.rs"), PathBuf::from("/srv/app/src/main.rs"));
    }

    #[test]
    fn relative_path_without_working_directory_is_relative() {
        let cfg = FileConfig::default();
        assert_eq!(cfg.resolve_path("src/main.rs"), PathBuf::from("src/main.rs"));
    }

    #[test]
    fn null_falls_back_to_default() {
        let cfg = FileConfig::from_value(&serde_json::Value::Null);
        assert!(cfg.working_directory.is_none());
    }

    #[test]
    fn to_value_roundtrips_working_directory() {
        let cfg = FileConfig {
            working_directory: Some("/srv/app".into()),
        };
        let v = cfg.to_value();
        assert_eq!(
            v,
            serde_json::json!({ "workingDirectory": "/srv/app" })
        );
        // Round-trip back through from_value preserves the field.
        assert_eq!(FileConfig::from_value(&v).working_directory, cfg.working_directory);
    }

    #[test]
    fn to_value_default_is_empty_object() {
        // Default has no fields populated, so to_value() must serialize to `{}`
        // (not `null`) so the JSON column round-trips through from_value.
        let v = FileConfig::default().to_value();
        assert_eq!(v, serde_json::json!({}));
        assert_eq!(FileConfig::from_value(&v), FileConfig::default());
    }
}
