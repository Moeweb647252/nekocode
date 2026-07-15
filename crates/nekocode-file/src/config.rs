use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Typed configuration for the `tool` middleware. Deserialized from the
/// per-thread `Middleware.config` JSON column (`{}` by default). Mirrors the
/// shape of `nekocode_shell::config::ShellConfig`.
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

    /// Resolve a caller-supplied path and verify it stays within the configured
    /// working directory (sandbox). Returns the resolved, canonicalized path on
    /// success.
    ///
    /// When `working_directory` is set, the resolved path must be equal to or a
    /// descendant of the (canonicalized) working directory after resolving `..`
    /// and symlinks. When `working_directory` is not set, there is no sandbox
    /// boundary to enforce, so the path is returned as-is after a best-effort
    /// canonicalization.
    ///
    /// For paths that don't yet exist on disk (e.g. a file about to be written),
    /// the parent directory is canonicalized instead and the check is applied
    /// there.
    pub fn resolve_and_check(&self, p: &str) -> Result<PathBuf, String> {
        let resolved = self.resolve_path(p);
        let Some(base) = &self.working_directory else {
            // No sandbox configured — return the resolved path. Best-effort
            // canonicalize; if the path doesn't exist, return it as-is.
            return Ok(resolved.canonicalize().unwrap_or(resolved));
        };
        let base_canon = Path::new(base)
            .canonicalize()
            .map_err(|e| format!("working directory '{}' cannot be canonicalized: {e}", base))?;
        // Try to canonicalize the resolved path. If it doesn't exist yet (e.g.
        // write_file creating a new file), canonicalize the parent instead.
        let checked = match resolved.canonicalize() {
            Ok(canon) => canon,
            Err(_) => {
                // Path doesn't exist. Canonicalize the nearest existing ancestor.
                let parent = resolved.parent();
                match parent {
                    Some(p) if !p.as_os_str().is_empty() => p
                        .canonicalize()
                        .map_err(|e| format!("parent path '{}' cannot be canonicalized: {e}", p.display()))?,
                    _ => {
                        return Err(format!(
                            "path '{}' resolves outside the working directory '{}'",
                            resolved.display(),
                            base_canon.display()
                        ));
                    }
                }
            }
        };
        if checked == base_canon || checked.starts_with(&base_canon) {
            Ok(resolved)
        } else {
            Err(format!(
                "path '{}' is outside the working directory '{}'",
                resolved.display(),
                base_canon.display()
            ))
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

    // ── resolve_and_check sandbox tests ──

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp_dir() -> PathBuf {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nekocode_file_config_sandbox_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sandbox_allows_descendant() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        // Existing file within sandbox.
        let file_path = dir.join("test.txt");
        std::fs::write(&file_path, "ok").unwrap();
        let result = cfg.resolve_and_check("test.txt");
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn sandbox_rejects_absolute_escape() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        let result = cfg.resolve_and_check("/etc/passwd");
        assert!(result.is_err(), "expected rejection for absolute path outside sandbox");
    }

    #[test]
    fn sandbox_rejects_dotdot_traversal() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        let result = cfg.resolve_and_check("../../etc/passwd");
        assert!(result.is_err(), "expected rejection for .. traversal outside sandbox");
    }

    #[test]
    fn sandbox_allows_new_file_in_existing_parent() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        // "new_file.txt" doesn't exist, but the parent (working directory) does.
        let result = cfg.resolve_and_check("new_file.txt");
        assert!(result.is_ok(), "expected Ok for new file in existing parent, got {:?}", result);
    }

    #[test]
    fn sandbox_no_working_directory_allows_anything() {
        let cfg = FileConfig::default();
        // Without a working directory, any path is allowed.
        let result = cfg.resolve_and_check("/etc/hosts");
        assert!(result.is_ok(), "expected Ok without sandbox, got {:?}", result);
    }
}
