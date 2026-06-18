use std::path::PathBuf;

use crate::api::prelude::*;
use nekocode_skills::SkillSource;

/// Spec-shaped skill metadata returned by `GET /api/util/skills`.
/// Mirrors the agentskills.io frontmatter fields so the UI can show
/// the same information any agentskills-compatible tool would.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub source: String, // "builtin" | "user"
    /// Path to the SKILL.md file on disk; absent for builtins.
    pub path: Option<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub allowed_tools: Option<String>,
}

/// GET /api/util/skills — list all available skills (builtin + user).
pub async fn list_skills(State(state): State<AppState>) -> ApiResult {
    let skills_dir = {
        let config = state.config.read().await;
        PathBuf::from(config.skills.directory.clone())
    };
    let skills = nekocode_skills::probe_skills(skills_dir).await;
    let mut items: Vec<SkillInfo> = skills
        .into_iter()
        .map(|s| {
            let (source, path) = match &s.source {
                SkillSource::Builtin => ("builtin".to_string(), None),
                SkillSource::UserDefined { manifest_path } => {
                    ("user".to_string(), Some(manifest_path.display().to_string()))
                }
            };
            SkillInfo {
                name: s.name,
                description: s.description,
                source,
                path,
                license: s.license,
                compatibility: s.compatibility,
                allowed_tools: s.allowed_tools,
            }
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    ApiResponse::ok(items)
}
