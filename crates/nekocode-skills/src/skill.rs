use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Priority of a skill prompt — controls injection order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillPriority {
    High,
    Medium,
    Low,
}

impl Default for SkillPriority {
    fn default() -> Self {
        SkillPriority::Medium
    }
}

impl fmt::Display for SkillPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillPriority::High => write!(f, "high"),
            SkillPriority::Medium => write!(f, "medium"),
            SkillPriority::Low => write!(f, "low"),
        }
    }
}

/// Where a skill comes from.
#[derive(Debug, Clone)]
pub enum SkillSource {
    /// Compiled into the binary.
    Builtin,
    /// Loaded from a user-provided SKILL.md file.
    UserDefined { path: PathBuf },
}

/// A parsed skill definition from a SKILL.md file.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: Option<String>,
    pub trigger: Option<String>,
    pub priority: SkillPriority,
    /// The Markdown body (frontmatter stripped).
    pub prompt: String,
    pub source: SkillSource,
}

// ---- YAML frontmatter intermediate (for deserialization) ----

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    trigger: Option<String>,
    priority: Option<SkillPriority>,
}

/// Parse a SKILL.md string into a `Skill`.
///
/// Expected format:
/// ```markdown
/// ---
/// name: skill-name
/// description: What it does
/// trigger: keyword|pattern
/// priority: high
/// ---
/// # Skill Name
/// Prompt body...
/// ```
///
/// Returns `None` if no (or empty) frontmatter block is found.
pub fn parse_skill_md(content: &str) -> anyhow::Result<Skill> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        anyhow::bail!("SKILL.md must start with a YAML frontmatter block (---)");
    }
    let rest = trimmed.strip_prefix("---").unwrap();
    let end = rest.find("\n---").ok_or_else(|| {
        anyhow::anyhow!("SKILL.md frontmatter missing closing ---")
    })?;
    let yaml_str = &rest[..end];
    let body = rest[end + 4..].trim();

    let fm: SkillFrontmatter =
        serde_yaml::from_str(yaml_str).map_err(|e| anyhow::anyhow!("Invalid YAML frontmatter: {e}"))?;

    let name = fm
        .name
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter missing required field: name"))?;

    Ok(Skill {
        name,
        description: fm.description,
        trigger: fm.trigger,
        priority: fm.priority.unwrap_or_default(),
        prompt: body.to_string(),
        source: SkillSource::Builtin,
    })
}

// Tests ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SKILL: &str = "---
name: test-skill
description: A test skill
trigger: test|check
priority: high
---

# Test Skill

This is the prompt body.";

    #[test]
    fn parse_valid_skill() {
        let s = parse_skill_md(VALID_SKILL).unwrap();
        assert_eq!(s.name, "test-skill");
        assert_eq!(s.description.unwrap(), "A test skill");
        assert_eq!(s.trigger.unwrap(), "test|check");
        assert_eq!(s.priority, SkillPriority::High);
        assert!(s.prompt.contains("Test Skill"));
        assert!(s.prompt.contains("prompt body"));
    }

    #[test]
    fn parse_minimal_skill() {
        let s = parse_skill_md(
            "---
name: minimal
---

Just the body.",
        )
        .unwrap();
        assert_eq!(s.name, "minimal");
        assert!(s.description.is_none());
        assert_eq!(s.priority, SkillPriority::Medium);
    }

    #[test]
    fn parse_requires_name() {
        let err = parse_skill_md(
            "---
description: no name here
---

Body",
        )
        .unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn rejects_no_frontmatter() {
        let err = parse_skill_md("Just plain text").unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn rejects_unclosed_frontmatter() {
        let err = parse_skill_md(
            "---
name: oops",
        )
        .unwrap_err();
        assert!(err.to_string().contains("closing ---"));
    }
}