use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProfile {
    pub name: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub allow_nested: bool,
    #[serde(default)]
    pub middlewares: Vec<String>,
}

/// Top-level shape of an `agents.toml` file: a single `[[agents]]` array.
/// (TOML's root is always a table, so the array lives under the `agents` key.)
#[derive(Debug, Deserialize)]
struct AgentsFile {
    #[serde(default)]
    agents: Vec<SubagentProfile>,
}

/// Catalog of profiles keyed by name, loaded from global + workspace
/// `agents.toml` files (workspace wholly replaces same-named global entries).
#[derive(Debug)]
pub struct ProfileCatalog {
    pub profiles: HashMap<String, SubagentProfile>,
}

impl ProfileCatalog {
    pub fn empty() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Load global then workspace, merging by name (workspace replaces).
    pub fn load(global_path: &Path, workspace_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let mut profiles: HashMap<String, SubagentProfile> = HashMap::new();
        // Global first (missing file is OK → empty catalog).
        if let Ok(content) = std::fs::read_to_string(global_path) {
            let parsed: AgentsFile = toml::from_str(&content)?;
            let mut seen = std::collections::HashSet::new();
            for p in parsed.agents {
                if p.name.is_empty() {
                    anyhow::bail!("profile with empty name in {}", global_path.display());
                }
                if !seen.insert(p.name.clone()) {
                    anyhow::bail!("duplicate profile name '{}' in {}", p.name, global_path.display());
                }
                profiles.insert(p.name.clone(), p);
            }
        }
        // Workspace second (missing is OK → skip). Wholly replaces same-named
        // global entries; intra-workspace duplicates are an error.
        if let Some(ws) = workspace_path {
            if let Ok(content) = std::fs::read_to_string(ws) {
                let parsed: AgentsFile = toml::from_str(&content)?;
                let mut seen = std::collections::HashSet::new();
                for p in parsed.agents {
                    if p.name.is_empty() {
                        anyhow::bail!("profile with empty name in {}", ws.display());
                    }
                    if !seen.insert(p.name.clone()) {
                        anyhow::bail!("duplicate profile name '{}' in {}", p.name, ws.display());
                    }
                    profiles.insert(p.name.clone(), p);
                }
            }
        }
        Ok(Self { profiles })
    }

    pub fn get(&self, name: &str) -> Result<&SubagentProfile, anyhow::Error> {
        self.profiles
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nekocode_subagent_profile_{}_{}_{}",
            std::process::id(),
            name,
            std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agents.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn load_missing_global_returns_empty_catalog() {
        let cat = ProfileCatalog::load(
            Path::new("/nonexistent/agents.toml"),
            None,
        )
        .expect("missing global is ok");
        assert!(cat.profiles.is_empty());
    }

    #[test]
    fn load_global_only() {
        let g = write_tmp(
            "global_only",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell", "tool"]
"#,
        );
        let cat = ProfileCatalog::load(&g, None).unwrap();
        let p = cat.get("explorer").unwrap();
        assert_eq!(p.middlewares, vec!["shell".to_string(), "tool".to_string()]);
        assert!(!p.allow_nested);
    }

    #[test]
    fn workspace_wholly_replaces_same_named_global() {
        let g = write_tmp(
            "replace_global",
            r#"
[[agents]]
name = "explorer"
system_prompt = "global prompt"
middlewares = ["shell", "tool"]
"#,
        );
        let w = write_tmp(
            "replace_ws",
            r#"
[[agents]]
name = "explorer"
middlewares = ["tool"]
allow_nested = true
"#,
        );
        let cat = ProfileCatalog::load(&g, Some(&w)).unwrap();
        let p = cat.get("explorer").unwrap();
        // Replaced wholesale: global system_prompt gone, workspace fields used.
        assert_eq!(p.system_prompt, None);
        assert_eq!(p.middlewares, vec!["tool".to_string()]);
        assert!(p.allow_nested);
    }

    #[test]
    fn workspace_adds_distinct_names() {
        let g = write_tmp(
            "add_global",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell"]
"#,
        );
        let w = write_tmp(
            "add_ws",
            r#"
[[agents]]
name = "reviewer"
middlewares = ["tool"]
"#,
        );
        let cat = ProfileCatalog::load(&g, Some(&w)).unwrap();
        assert!(cat.get("explorer").is_ok());
        assert!(cat.get("reviewer").is_ok());
    }

    #[test]
    fn duplicate_name_in_single_file_is_error() {
        let g = write_tmp(
            "dup",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell"]

[[agents]]
name = "explorer"
middlewares = ["tool"]
"#,
        );
        let err = ProfileCatalog::load(&g, None).expect_err("duplicate should error");
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn empty_name_is_error() {
        let g = write_tmp(
            "emptyname",
            r#"
[[agents]]
name = ""
middlewares = ["shell"]
"#,
        );
        let err = ProfileCatalog::load(&g, None).expect_err("empty name should error");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn get_unknown_returns_error() {
        let cat = ProfileCatalog::empty();
        let err = cat.get("nope").expect_err("unknown profile");
        assert!(err.to_string().contains("not found"));
    }
}
