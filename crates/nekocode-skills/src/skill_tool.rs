//! Tools that implement progressive disclosure for skills.
//!
//! Per <https://agentskills.io/specification>, only `name + description`
//! ship in the catalog (Tier 1). The full SKILL.md body and bundled
//! resource files are loaded on demand by the agent through these tools:
//!
//! - [`ReadSkillTool`]: returns the full SKILL.md body of an enabled
//!   skill (Tier 2).
//! - [`ReadSkillFileTool`]: returns the contents of a file under the
//!   skill's directory — `scripts/*`, `references/*`, `assets/*`, etc.
//!   (Tier 3).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::skill::Skill;

/// Tier 2 — return a skill's full SKILL.md body. Only enabled skills are
/// addressable; unknown names produce an `InvalidParameters` error so the
/// model gets useful feedback.
pub struct ReadSkillTool {
    pub skills: Arc<HashMap<String, Skill>>,
}

#[async_trait::async_trait]
impl Tool for ReadSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_skill".to_string(),
            description:
                "Read the full instructions (SKILL.md body) of a skill from the catalog. Call this BEFORE acting on a task whenever a listed skill looks relevant. Returns the skill's name, description, and the entire Markdown body."
                    .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name as listed in the catalog."
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'name' parameter".into()))?;
        let skill = self.skills.get(name).ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "Unknown skill '{name}'. Only enabled skills are readable; check the catalog."
            ))
        })?;
        Ok(serde_json::json!({
            "name": skill.name,
            "description": skill.description,
            "body": skill.body,
        }))
    }
}

/// Tier 3 — read a file from a skill's on-disk directory. Path resolution
/// is constrained to the skill's `root` to prevent traversal escapes.
/// Builtin skills (no on-disk root) are not readable through this tool.
pub struct ReadSkillFileTool {
    pub skills: Arc<HashMap<String, Skill>>,
}

#[async_trait::async_trait]
impl Tool for ReadSkillFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_skill_file".to_string(),
            description:
                "Read a file from inside an enabled skill's directory (e.g. references/REFERENCE.md, scripts/extract.py, assets/template.html). Use a path relative to the skill root."
                    .to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": {
                        "type": "string",
                        "description": "The skill name as listed in the catalog."
                    },
                    "path": {
                        "type": "string",
                        "description": "Path relative to the skill root, e.g. \"references/REFERENCE.md\"."
                    }
                },
                "required": ["skill", "path"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'skill' parameter".into()))?;
        let rel_path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'path' parameter".into()))?;

        let skill = self.skills.get(skill_name).ok_or_else(|| {
            ToolError::InvalidParameters(format!("Unknown skill '{skill_name}'."))
        })?;
        let root = skill.root.as_deref().ok_or_else(|| {
            ToolError::ExecutionError(format!(
                "Skill '{skill_name}' is a builtin and has no on-disk files."
            ))
        })?;

        let target = root.join(rel_path);
        let resolved = resolve_within(root, &target).map_err(ToolError::InvalidParameters)?;

        let content = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to read {}: {e}", resolved.display()))
        })?;

        Ok(serde_json::json!({
            "skill": skill.name,
            "path": rel_path,
            "absolute_path": resolved.to_string_lossy(),
            "content": content,
        }))
    }
}

/// Canonicalize `target` and ensure it is the same as, or a descendant of,
/// the canonical `root`. Returns the canonical target on success; an
/// `InvalidParameters` message on traversal escape.
fn resolve_within(root: &std::path::Path, target: &std::path::Path) -> Result<PathBuf, String> {
    let root_canon = root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize skill root {}: {e}", root.display()))?;
    let target_canon = target.canonicalize().map_err(|e| {
        format!(
            "File not found inside skill root: {} ({e})",
            target.display()
        )
    })?;
    if !target_canon.starts_with(&root_canon) {
        return Err(format!(
            "Path escapes skill root: {} is not inside {}",
            target_canon.display(),
            root_canon.display()
        ));
    }
    Ok(target_canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::SkillSource;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nekocode_skill_tool_test_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_skill(name: &str, root: Option<PathBuf>, body: &str) -> Skill {
        Skill {
            name: name.into(),
            description: "x".into(),
            license: None,
            compatibility: None,
            metadata: HashMap::new(),
            allowed_tools: None,
            body: body.into(),
            source: SkillSource::Builtin,
            root,
        }
    }

    #[tokio::test]
    async fn read_skill_returns_body() {
        let mut map = HashMap::new();
        map.insert(
            "demo".to_string(),
            make_skill("demo", None, "Hello body."),
        );
        let tool = ReadSkillTool {
            skills: Arc::new(map),
        };
        let out = tool
            .call(serde_json::json!({ "name": "demo" }))
            .await
            .unwrap();
        assert_eq!(out["body"], "Hello body.");
        assert_eq!(out["name"], "demo");
    }

    #[tokio::test]
    async fn read_skill_unknown_name() {
        let tool = ReadSkillTool {
            skills: Arc::new(HashMap::new()),
        };
        let err = tool
            .call(serde_json::json!({ "name": "nope" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn read_skill_file_reads_under_root() {
        let root = temp_dir();
        let refs = root.join("references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("a.md"), "ref content").unwrap();

        let mut map = HashMap::new();
        map.insert(
            "demo".to_string(),
            make_skill("demo", Some(root.clone()), ""),
        );
        let tool = ReadSkillFileTool {
            skills: Arc::new(map),
        };
        let out = tool
            .call(serde_json::json!({ "skill": "demo", "path": "references/a.md" }))
            .await
            .unwrap();
        assert_eq!(out["content"], "ref content");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn read_skill_file_blocks_traversal() {
        let root = temp_dir();
        std::fs::write(root.join("inside.txt"), "ok").unwrap();
        // Place a sibling file the traversal would otherwise reach.
        let parent = root.parent().unwrap();
        let outside = parent.join(format!(
            "outside_{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&outside, "leak").unwrap();

        let mut map = HashMap::new();
        map.insert(
            "demo".to_string(),
            make_skill("demo", Some(root.clone()), ""),
        );
        let tool = ReadSkillFileTool {
            skills: Arc::new(map),
        };

        let rel = format!("../{}", outside.file_name().unwrap().to_string_lossy());
        let err = tool
            .call(serde_json::json!({ "skill": "demo", "path": rel }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));

        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn read_skill_file_rejects_builtin() {
        let mut map = HashMap::new();
        map.insert(
            "demo".to_string(),
            make_skill("demo", None, ""),
        );
        let tool = ReadSkillFileTool {
            skills: Arc::new(map),
        };
        let err = tool
            .call(serde_json::json!({ "skill": "demo", "path": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionError(_)));
    }
}
