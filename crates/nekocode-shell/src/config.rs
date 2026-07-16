use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Typed configuration for the `shell` middleware. Deserialized from the
/// per-thread `Middleware.config` JSON column (`{}` by default).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShellConfig {
    /// Working directory applied to every spawned process. When `None`, the
    /// process inherits the server's current directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Override the shell executable. Defaults to `bash` on unix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Wall-clock timeout in seconds for the one-shot `shell` tool.
    /// `None` means no timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Extra environment variables applied to every spawned process, merged on
    /// top of the inherited environment.
    #[serde(default)]
    pub envs: HashMap<String, String>,
}

impl ShellConfig {
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

    /// Resolve the shell executable to invoke (`sh -c` / `bash -c`).
    pub fn program(&self) -> &str {
        self.shell.as_deref().unwrap_or("bash")
    }

    /// Apply working directory and env overrides to a command builder.
    pub fn apply(&self, cmd: &mut tokio::process::Command) {
        if let Some(wd) = self.working_directory.as_deref() {
            cmd.current_dir(wd);
        }
        for (k, v) in &self.envs {
            cmd.env(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_round_trip() {
        let cfg = ShellConfig::default();
        let v = cfg.to_value();
        let cfg2 = ShellConfig::from_value(&v);
        assert_eq!(cfg.working_directory, cfg2.working_directory);
        assert_eq!(cfg.shell, cfg2.shell);
        assert_eq!(cfg.timeout_secs, cfg2.timeout_secs);
        assert_eq!(cfg.envs, cfg2.envs);
    }

    #[test]
    fn full_config_round_trip() {
        let cfg = ShellConfig {
            working_directory: Some("/tmp".into()),
            shell: Some("zsh".into()),
            timeout_secs: Some(30),
            envs: HashMap::from([("FOO".into(), "bar".into()), ("BAZ".into(), "qux".into())]),
        };
        let v = cfg.to_value();
        // Verify camelCase serialization.
        assert!(v.get("workingDirectory").is_some());
        assert!(v.get("timeoutSecs").is_some());
        let cfg2 = ShellConfig::from_value(&v);
        assert_eq!(cfg.working_directory, cfg2.working_directory);
        assert_eq!(cfg.shell, cfg2.shell);
        assert_eq!(cfg.timeout_secs, cfg2.timeout_secs);
        assert_eq!(cfg.envs, cfg2.envs);
    }

    #[test]
    fn null_input_gives_default() {
        let cfg = ShellConfig::from_value(&serde_json::Value::Null);
        assert!(cfg.working_directory.is_none());
        assert!(cfg.shell.is_none());
        assert!(cfg.timeout_secs.is_none());
        assert!(cfg.envs.is_empty());
    }

    #[test]
    fn partial_config_round_trip() {
        let v = json!({ "workingDirectory": "/home", "envs": { "KEY": "val" } });
        let cfg = ShellConfig::from_value(&v);
        assert_eq!(cfg.working_directory.as_deref(), Some("/home"));
        assert!(cfg.shell.is_none());
        assert_eq!(cfg.envs["KEY"], "val");
    }
}
