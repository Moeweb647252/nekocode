pub mod config;
mod loader;
pub mod skill;

pub use config::SkillsConfig;
pub use loader::SkillLoader;
pub use skill::{Skill, SkillPriority, SkillSource};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nekocode_core::middleware::Middleware;
use tokio::sync::OnceCell;

/// Middleware that injects skill prompts into the agent's system prompt.
///
/// Skills are loaded lazily on the first `before_generate` call (via
/// [`SkillLoader`]) from both the compiled-in builtin set and the user's
/// configured skills directory.
pub struct SkillsMiddleware {
    config: Arc<SkillsConfig>,
    /// Lazily-loaded name → Skill map, shared across calls.
    skills: OnceCell<HashMap<String, Skill>>,
    skills_dir: PathBuf,
}

impl SkillsMiddleware {
    pub fn new(config: SkillsConfig, skills_dir: PathBuf) -> Self {
        Self {
            config: Arc::new(config),
            skills: OnceCell::new(),
            skills_dir,
        }
    }

    async fn get_skills(&self) -> &HashMap<String, Skill> {
        self.skills
            .get_or_init(|| async {
                let loader = SkillLoader::new(self.skills_dir.clone());
                loader.load_all().await
            })
            .await
    }
}

#[async_trait::async_trait]
impl Middleware for SkillsMiddleware {
    async fn before_generate(
        &self,
        request: &mut nekocode_core::types::GenerateRequest,
        _registry: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        let skills = self.get_skills().await;
        let enabled: Vec<&Skill> = self
            .config
            .enabled
            .iter()
            .filter_map(|name| skills.get(name))
            .collect();

        if enabled.is_empty() {
            return Ok(());
        }

        // Sort by priority order: High, Medium, Low.
        let mut sorted = enabled;
        sorted.sort_by_key(|s| match s.priority {
            SkillPriority::High => 0,
            SkillPriority::Medium => 1,
            SkillPriority::Low => 2,
        });

        let mut skill_blocks: Vec<String> = Vec::with_capacity(sorted.len());
        for s in &sorted {
            skill_blocks.push(format!("## Skill: {}\n\n{}", s.name, s.prompt));
        }
        let skills_prompt = skill_blocks.join("\n\n");

        // Prepend to the existing system prompt.
        let existing = request.system_prompt.take().unwrap_or_default();
        request.system_prompt = if existing.is_empty() {
            Some(skills_prompt)
        } else {
            Some(format!("{skills_prompt}\n\n{existing}"))
        };

        Ok(())
    }
}

/// Load all skills (builtin + user-defined) and return them as a flat Vec.
/// Used by the settings UI's probe/list API.
pub async fn probe_skills(skills_dir: PathBuf) -> Vec<Skill> {
    let loader = SkillLoader::new(skills_dir);
    loader
        .load_all()
        .await
        .into_values()
        .collect()
}