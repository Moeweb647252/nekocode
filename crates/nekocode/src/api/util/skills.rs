use std::path::PathBuf;

use crate::api::prelude::*;
use nekocode_skills::SkillSource;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: Option<String>,
    pub priority: String,
    pub source: String,
    pub path: Option<String>,
}

/// GET /api/util/skills — list all available skills (builtin + user-defined).
pub async fn list_skills(State(state): State<AppState>) -> ApiResult {
    let skills_dir = {
        let config = state.config.read().await;
        PathBuf::from(config.skills.directory.clone())
    };
    let skills = nekocode_skills::probe_skills(skills_dir).await;
    let items: Vec<SkillInfo> = skills
        .into_iter()
        .map(|s| {
            let (source, path) = match &s.source {
                SkillSource::Builtin => ("builtin".to_string(), None),
                SkillSource::UserDefined { path } => {
                    ("user".to_string(), Some(path.display().to_string()))
                }
            };
            SkillInfo {
                name: s.name,
                description: s.description,
                priority: s.priority.to_string(),
                source,
                path,
            }
        })
        .collect();
    ApiResponse::ok(items)
}