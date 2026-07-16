//! Discovers spec-compliant skills on disk and from the builtin set.
//!
//! Per <https://agentskills.io/specification>, a skill is a directory with
//! a `SKILL.md` file. Single-file skills are not supported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::skill::{Skill, SkillSource, parse_skill_md};

/// Builtin skill content compiled into the binary, keyed by skill name.
/// Builtins are prompt-only — they have no on-disk root, so they cannot
/// expose scripts or reference files.
const BUILTIN_SKILLS: &[(&str, &str)] = &[
    (
        "skill-creator",
        include_str!("builtin/skill-creator/SKILL.md"),
    ),
    (
        "skill-installer",
        include_str!("builtin/skill-installer/SKILL.md"),
    ),
];

/// Loads spec-compliant skills from the builtin set and a user skills
/// directory. User-defined skills override builtins with the same name.
pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    /// Construct a loader reading user-defined skills from `skills_dir`.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Parse all builtin skills plus every spec-compliant subdirectory of
    /// the configured skills dir, keyed by skill `name`. User-defined
    /// skills override builtins with the same name.
    pub async fn load_all(&self) -> HashMap<String, Skill> {
        let mut skills = HashMap::new();

        // 1. Builtins. Builtin SKILL.md content is trusted to match its
        //    declared key, but we still parse it to fail loud on
        //    mistakes during development.
        for (name, content) in BUILTIN_SKILLS {
            match parse_skill_md(content, Some(name)) {
                Ok(mut s) => {
                    s.source = SkillSource::Builtin;
                    s.root = None;
                    skills.insert(s.name.clone(), s);
                }
                Err(e) => warn!("Failed to parse builtin skill '{name}': {e}"),
            }
        }

        // 2. User-defined skills. Each subdirectory containing a SKILL.md
        //    is a skill; loose `.md` files are ignored (the spec mandates
        //    directory layout so scripts/references/assets paths resolve).
        if !self.skills_dir.exists() {
            return skills;
        }
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
                    let metadata = match entry.metadata().await {
                        Ok(m) => m,
                        Err(e) => {
                            warn!("Failed to stat {}: {e}", path.display());
                            continue;
                        }
                    };
                    if !metadata.is_dir() {
                        continue;
                    }
                    if let Some(skill) = load_skill_directory(&path).await {
                        skills.insert(skill.name.clone(), skill);
                    }
                }
                Err(e) => {
                    warn!("Error iterating skills directory: {e}");
                    break;
                }
            }
        }

        skills
    }
}

/// Load one skill from `<dir>/SKILL.md`. The directory's basename is
/// passed to the parser to enforce the spec's `name == parent dir` rule.
async fn load_skill_directory(dir: &Path) -> Option<Skill> {
    let manifest_path = dir.join("SKILL.md");
    if !manifest_path.exists() {
        return None;
    }
    let dir_name = match dir.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => {
            warn!(
                "Skipping skill directory with non-utf8 name: {}",
                dir.display()
            );
            return None;
        }
    };
    let content = match tokio::fs::read_to_string(&manifest_path).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read {}: {e}", manifest_path.display());
            return None;
        }
    };
    let mut skill = match parse_skill_md(&content, Some(dir_name)) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to parse {}: {e}", manifest_path.display());
            return None;
        }
    };
    skill.source = SkillSource::UserDefined { manifest_path };
    skill.root = Some(dir.to_path_buf());
    Some(skill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_skills_dir() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nekocode_skills_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("create temp skills dir");
        dir
    }

    #[tokio::test]
    async fn loads_builtins_when_dir_missing() {
        let dir = temp_skills_dir();
        std::fs::remove_dir_all(&dir).ok();
        let loader = SkillLoader::new(dir);
        let skills = loader.load_all().await;
        assert!(skills.contains_key("skill-creator"));
        assert!(skills.contains_key("skill-installer"));
    }

    #[tokio::test]
    async fn loads_directory_skill() {
        let root = temp_skills_dir();
        let skill_dir = root.join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: x\n---\nbody",
        )
        .unwrap();

        let skills = SkillLoader::new(root.clone()).load_all().await;
        let s = skills.get("my-skill").expect("loaded");
        assert_eq!(s.root.as_deref(), Some(skill_dir.as_path()));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn rejects_skill_with_mismatched_dir_name() {
        let root = temp_skills_dir();
        let skill_dir = root.join("dir-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: different-name\ndescription: x\n---\nbody",
        )
        .unwrap();

        let skills = SkillLoader::new(root.clone()).load_all().await;
        // Both names should be absent: parser rejects mismatch.
        assert!(!skills.contains_key("dir-name"));
        assert!(!skills.contains_key("different-name"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn ignores_loose_md_files() {
        let root = temp_skills_dir();
        std::fs::write(
            root.join("legacy.md"),
            "---\nname: legacy\ndescription: x\n---\nbody",
        )
        .unwrap();
        let skills = SkillLoader::new(root.clone()).load_all().await;
        assert!(!skills.contains_key("legacy"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn skips_dirs_without_skill_md() {
        let root = temp_skills_dir();
        std::fs::create_dir_all(root.join("not-a-skill")).unwrap();
        let skills = SkillLoader::new(root.clone()).load_all().await;
        assert!(!skills.contains_key("not-a-skill"));
        std::fs::remove_dir_all(&root).ok();
    }
}
