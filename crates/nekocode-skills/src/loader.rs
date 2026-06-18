use std::collections::HashMap;
use std::path::PathBuf;

use tracing::warn;

use crate::skill::{parse_skill_md, Skill, SkillSource};

/// Builtin skill content compiled into the binary. Each entry is
/// `(name, raw_md_content)`.
const BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("skill-creator", include_str!("skill_creator.md")),
    ("skill-installer", include_str!("skill_installer.md")),
];

/// Loads skills from the builtin set and the user skills directory.
pub struct SkillLoader {
    /// Filesystem directory containing user-defined SKILL.md files.
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Load all available skills (builtin + user-defined) into a name→Skill map.
    /// User-defined skills override builtins with the same name.
    pub async fn load_all(&self) -> HashMap<String, Skill> {
        let mut skills = HashMap::new();

        // 1. Load builtins.
        for (name, content) in BUILTIN_SKILLS {
            match parse_skill_md(content) {
                Ok(mut s) => {
                    s.source = SkillSource::Builtin;
                    skills.insert(s.name.clone(), s);
                }
                Err(e) => {
                    warn!("Failed to parse builtin skill '{name}': {e}");
                }
            }
        }

        // 2. Load user-defined skills from the configured directory.
        if self.skills_dir.exists() {
            let mut entries = match tokio::fs::read_dir(&self.skills_dir).await {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        "Failed to read skills directory {}: {e}",
                        self.skills_dir.display()
                    );
                    return skills;
                }
            };
            loop {
                match entries.next_entry().await {
                    Ok(None) => break,
                    Ok(Some(entry)) => {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("md") {
                            continue;
                        }
                        match tokio::fs::read_to_string(&path).await {
                            Ok(content) => match parse_skill_md(&content) {
                                Ok(mut s) => {
                                    s.source = SkillSource::UserDefined {
                                        path: path.clone(),
                                    };
                                    skills.insert(s.name.clone(), s);
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to parse user skill {}: {e}",
                                        path.display()
                                    );
                                }
                            },
                            Err(e) => {
                                warn!("Failed to read user skill {}: {e}", path.display());
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Error iterating skills directory: {e}");
                        break;
                    }
                }
            }
        }

        skills
    }
}
