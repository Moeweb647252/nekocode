//! nekocode-skills — agentskills.io-compliant skills middleware.
//!
//! Implements progressive disclosure per
//! <https://agentskills.io/specification>:
//!
//! - **Tier 1 (catalog)**: every enabled skill's `name` and `description`
//!   are injected into the agent's system prompt at the top of each
//!   generation.
//! - **Tier 2 (instructions)**: the SKILL.md body is fetched on demand
//!   when the model calls the `read_skill` tool.
//! - **Tier 3 (resources)**: files inside the skill directory
//!   (`scripts/`, `references/`, `assets/`) are fetched on demand when
//!   the model calls the `read_skill_file` tool.
//!
//! Scripts are not spawned by this middleware — the model invokes them
//! using whatever shell/tool middleware the user has separately enabled,
//! which is the canonical agentskills.io approach.

pub mod config;
mod loader;
pub mod skill;
pub mod skill_tool;

pub use config::SkillsConfig;
pub use loader::SkillLoader;
pub use skill::{Skill, SkillSource};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nekocode_core::middleware::Middleware;
use tokio::sync::OnceCell;

/// Per-thread skills middleware. Loads skills lazily on first
/// `before_generate`, then on every generation injects the enabled
/// skills' catalog into the system prompt and registers the
/// `read_skill` / `read_skill_file` tools that implement progressive
/// disclosure.
pub struct SkillsMiddleware {
    config: Arc<SkillsConfig>,
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
        registry: &mut nekocode_types::tool::ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        let skills = self.get_skills().await;

        // Build the enabled set in one pass so both the catalog text and
        // the tool registration share an identical view.
        let enabled: HashMap<String, Skill> = self
            .config
            .enabled
            .iter()
            .filter_map(|name| skills.get(name).map(|s| (name.clone(), s.clone())))
            .collect();

        if enabled.is_empty() {
            return Ok(());
        }

        // Tier-1 catalog: name + description (+ root hint when on disk).
        let mut entries: Vec<&Skill> = enabled.values().collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let body: String = entries
            .iter()
            .map(|s| {
                let mut block = format!("- name: {}\n  description: {}", s.name, s.description);
                if let Some(root) = &s.root {
                    block.push_str(&format!("\n  root: {}", root.display()));
                }
                block
            })
            .collect::<Vec<_>>()
            .join("\n");

        let catalog = format!(
            "The following skills are available. Each entry is a short \
catalog summary; the full instructions live in the skill's SKILL.md \
body and bundled files. BEFORE acting on a task that any listed skill \
is relevant for, call `read_skill` with that skill's name to load its \
full instructions. Use `read_skill_file` to read referenced files \
under the skill's root (e.g. references/REFERENCE.md, scripts/extract.py).\n\n{body}"
        );

        let existing = request.system_prompt.take().unwrap_or_default();
        request.system_prompt = if existing.is_empty() {
            Some(catalog)
        } else {
            Some(format!("{catalog}\n\n{existing}"))
        };

        // Register the two progressive-disclosure tools. They share an
        // `Arc<HashMap>` snapshot of just the enabled skills so the model
        // can't reach un-enabled ones via either tool.
        let enabled_arc = Arc::new(enabled);
        registry.insert(
            "read_skill".into(),
            Arc::new(skill_tool::ReadSkillTool {
                skills: enabled_arc.clone(),
            }),
        );
        registry.insert(
            "read_skill_file".into(),
            Arc::new(skill_tool::ReadSkillFileTool {
                skills: enabled_arc,
            }),
        );

        Ok(())
    }
}

/// Load every available skill (builtin + user-defined). Used by the
/// settings UI to populate the catalog list.
pub async fn probe_skills(skills_dir: PathBuf) -> Vec<Skill> {
    SkillLoader::new(skills_dir)
        .load_all()
        .await
        .into_values()
        .collect()
}
