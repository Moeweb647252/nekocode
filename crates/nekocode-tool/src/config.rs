use std::path::{Path, PathBuf};

/// Typed configuration for the `tool` middleware. Deserialized from the
/// per-thread `Middleware.config` JSON column (`{}` by default). Mirrors the
/// shape of [`nekocode_shell::config::ShellConfig`].
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileConfig {
    /// Working directory used to resolve relative paths. When `None`, relative
    /// paths resolve against the server's current directory.
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
}
