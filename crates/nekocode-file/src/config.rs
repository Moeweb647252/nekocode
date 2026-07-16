use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
pub(crate) enum ResolvedFile {
    Ambient(PathBuf),
    Sandboxed {
        base: PathBuf,
        relative: PathBuf,
        display: PathBuf,
    },
}

impl ResolvedFile {
    pub(crate) fn display_path(&self) -> &Path {
        match self {
            Self::Ambient(path) => path,
            Self::Sandboxed { display, .. } => display,
        }
    }
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

    /// Resolve a path into either ambient access or a capability-relative path.
    /// Sandboxed callers must perform I/O through `cap_std::fs::Dir`; returning
    /// the relative path instead of an already-checked ambient path removes the
    /// symlink-swap window between validation and open.
    pub(crate) fn resolve_for_io(&self, p: &str) -> Result<ResolvedFile, String> {
        let Some(base) = &self.working_directory else {
            return Ok(ResolvedFile::Ambient(self.resolve_path(p)));
        };
        let base_canon = Path::new(base)
            .canonicalize()
            .map_err(|e| format!("working directory '{}' cannot be canonicalized: {e}", base))?;
        let input = Path::new(p);
        let relative = if input.is_absolute() {
            input.strip_prefix(&base_canon).map_err(|_| {
                format!(
                    "path '{}' is outside the working directory '{}'",
                    input.display(),
                    base_canon.display()
                )
            })?
        } else {
            input
        };
        let relative = relative.to_path_buf();
        let mut depth = 0usize;
        for component in relative.components() {
            match component {
                std::path::Component::Normal(_) => depth += 1,
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir if depth > 0 => depth -= 1,
                std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_) => {
                    return Err(format!(
                        "path '{}' escapes the working directory '{}'",
                        input.display(),
                        base_canon.display()
                    ));
                }
            }
        }
        let display = base_canon.join(&relative);
        Ok(ResolvedFile::Sandboxed {
            base: base_canon,
            relative,
            display,
        })
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
        assert_eq!(
            cfg.resolve_path("src/main.rs"),
            PathBuf::from("/srv/app/src/main.rs")
        );
    }

    #[test]
    fn relative_path_without_working_directory_is_relative() {
        let cfg = FileConfig::default();
        assert_eq!(
            cfg.resolve_path("src/main.rs"),
            PathBuf::from("src/main.rs")
        );
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
        assert_eq!(v, serde_json::json!({ "workingDirectory": "/srv/app" }));
        // Round-trip back through from_value preserves the field.
        assert_eq!(
            FileConfig::from_value(&v).working_directory,
            cfg.working_directory
        );
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
        let result = cfg.resolve_for_io("test.txt");
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn sandbox_rejects_absolute_escape() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        let result = cfg.resolve_for_io("/etc/passwd");
        assert!(
            result.is_err(),
            "expected rejection for absolute path outside sandbox"
        );
    }

    #[test]
    fn sandbox_rejects_dotdot_traversal() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        let result = cfg.resolve_for_io("../../etc/passwd");
        assert!(
            result.is_err(),
            "expected rejection for .. traversal outside sandbox"
        );
    }

    #[test]
    fn sandbox_allows_new_file_in_existing_parent() {
        let dir = tmp_dir();
        let cfg = FileConfig {
            working_directory: Some(dir.to_string_lossy().to_string()),
        };
        // "new_file.txt" doesn't exist, but the parent (working directory) does.
        let result = cfg.resolve_for_io("new_file.txt");
        assert!(
            result.is_ok(),
            "expected Ok for new file in existing parent, got {:?}",
            result
        );
    }

    #[test]
    fn sandbox_no_working_directory_allows_anything() {
        let cfg = FileConfig::default();
        // Without a working directory, any path is allowed.
        let result = cfg.resolve_for_io("/etc/hosts");
        assert!(
            result.is_ok(),
            "expected Ok without sandbox, got {:?}",
            result
        );
    }
}
