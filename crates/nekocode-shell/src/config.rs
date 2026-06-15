use std::collections::HashMap;
use serde::{Deserialize, Serialize};

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
