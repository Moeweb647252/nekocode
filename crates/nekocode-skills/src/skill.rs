//! Spec-compliant SKILL.md frontmatter + parser.
//!
//! Conforms to <https://agentskills.io/specification>. Only the fields
//! defined by the spec are recognized; private extensions are not parsed.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

/// Where a skill came from.
#[derive(Debug, Clone)]
pub enum SkillSource {
    /// Compiled into the binary.
    Builtin,
    /// Loaded from disk; `manifest_path` points at the SKILL.md.
    UserDefined { manifest_path: PathBuf },
}

/// A parsed, spec-compliant skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: HashMap<String, String>,
    pub allowed_tools: Option<String>,
    /// Markdown body after the frontmatter. Loaded into the agent's context
    /// only when the model calls `read_skill`.
    pub body: String,
    pub source: SkillSource,
    /// Skill root directory (parent of SKILL.md). `None` for builtins.
    pub root: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    metadata: Option<serde_yaml::Mapping>,
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Option<String>,
}

/// Parse a SKILL.md document into a [`Skill`].
///
/// `expected_name`, when provided, is checked against the parsed `name`
/// field — the spec requires the frontmatter `name` to match the parent
/// directory name. Pass `None` for builtin skills.
pub fn parse_skill_md(content: &str, expected_name: Option<&str>) -> anyhow::Result<Skill> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        anyhow::bail!("SKILL.md must start with a YAML frontmatter block (---)");
    }
    let rest = trimmed.strip_prefix("---").unwrap();
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter missing closing ---"))?;
    let yaml_str = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n').to_string();

    let fm: Frontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| anyhow::anyhow!("Invalid YAML frontmatter: {e}"))?;

    let name = fm
        .name
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter missing required field: name"))?;
    validate_name(&name)?;
    if let Some(expected) = expected_name {
        if name != expected {
            anyhow::bail!(
                "SKILL.md `name` ({name}) must match parent directory name ({expected})"
            );
        }
    }

    let description = fm
        .description
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter missing required field: description"))?;
    validate_description(&description)?;

    if let Some(c) = &fm.compatibility {
        validate_compatibility(c)?;
    }

    let metadata = fm.metadata.map(coerce_metadata).unwrap_or_default();

    Ok(Skill {
        name,
        description,
        license: fm.license,
        compatibility: fm.compatibility,
        metadata,
        allowed_tools: fm.allowed_tools,
        body,
        source: SkillSource::Builtin,
        root: None,
    })
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    let len = name.chars().count();
    if len == 0 {
        anyhow::bail!("`name` must be 1-64 characters (was empty)");
    }
    if len > 64 {
        anyhow::bail!("`name` must be at most 64 characters (was {len})");
    }
    if name.starts_with('-') || name.ends_with('-') {
        anyhow::bail!("`name` must not start or end with a hyphen: {name}");
    }
    if name.contains("--") {
        anyhow::bail!("`name` must not contain consecutive hyphens: {name}");
    }
    for c in name.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            anyhow::bail!(
                "`name` may only contain lowercase letters, digits, and hyphens: {name}"
            );
        }
    }
    Ok(())
}

fn validate_description(desc: &str) -> anyhow::Result<()> {
    if desc.trim().is_empty() {
        anyhow::bail!("`description` must be non-empty");
    }
    let len = desc.chars().count();
    if len > 1024 {
        anyhow::bail!("`description` must be at most 1024 characters (was {len})");
    }
    Ok(())
}

fn validate_compatibility(value: &str) -> anyhow::Result<()> {
    let len = value.chars().count();
    if len == 0 {
        anyhow::bail!("`compatibility` must be 1-500 characters when present");
    }
    if len > 500 {
        anyhow::bail!("`compatibility` must be at most 500 characters (was {len})");
    }
    Ok(())
}

/// Coerce a YAML mapping to `HashMap<String, String>`, matching the
/// canonical agentskills parser which stringifies both keys and values.
fn coerce_metadata(map: serde_yaml::Mapping) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(map.len());
    for (k, v) in map {
        out.insert(yaml_to_string(&k), yaml_to_string(&v));
    }
    out
}

fn yaml_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::Null => "null".into(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(c: &str) -> anyhow::Result<Skill> {
        parse_skill_md(c, None)
    }

    #[test]
    fn parses_minimal() {
        let s = parse("---\nname: t\ndescription: x\n---\nbody").unwrap();
        assert_eq!(s.name, "t");
        assert_eq!(s.description, "x");
        assert!(s.metadata.is_empty());
    }

    #[test]
    fn parses_full() {
        let s = parse(
            "---\nname: pdf-processing\ndescription: Extracts PDFs.\nlicense: Apache-2.0\ncompatibility: Requires Python 3.14+\nmetadata:\n  author: example-org\n  version: \"1.0\"\nallowed-tools: Bash(git:*) Read\n---\nbody",
        )
        .unwrap();
        assert_eq!(s.name, "pdf-processing");
        assert_eq!(s.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(s.compatibility.as_deref(), Some("Requires Python 3.14+"));
        assert_eq!(s.metadata.get("author").map(String::as_str), Some("example-org"));
        assert_eq!(s.metadata.get("version").map(String::as_str), Some("1.0"));
        assert_eq!(s.allowed_tools.as_deref(), Some("Bash(git:*) Read"));
    }

    #[test]
    fn rejects_missing_description() {
        let e = parse("---\nname: t\n---\nbody").unwrap_err().to_string();
        assert!(e.contains("description"));
    }

    #[test]
    fn rejects_empty_description() {
        let e = parse("---\nname: t\ndescription: '   '\n---\nbody")
            .unwrap_err()
            .to_string();
        assert!(e.contains("description"));
    }

    #[test]
    fn rejects_long_description() {
        let long = "a".repeat(1025);
        let src = format!("---\nname: t\ndescription: {long}\n---\nbody");
        let e = parse(&src).unwrap_err().to_string();
        assert!(e.contains("1024"));
    }

    #[test]
    fn rejects_uppercase_name() {
        let e = parse("---\nname: PDF\ndescription: x\n---\nbody")
            .unwrap_err()
            .to_string();
        assert!(e.contains("lowercase"));
    }

    #[test]
    fn rejects_leading_hyphen_name() {
        let e = parse("---\nname: -foo\ndescription: x\n---\nbody")
            .unwrap_err()
            .to_string();
        assert!(e.contains("hyphen"));
    }

    #[test]
    fn rejects_consecutive_hyphens() {
        let e = parse("---\nname: foo--bar\ndescription: x\n---\nbody")
            .unwrap_err()
            .to_string();
        assert!(e.contains("consecutive"));
    }

    #[test]
    fn rejects_long_name() {
        let long = "a".repeat(65);
        let src = format!("---\nname: {long}\ndescription: x\n---\nbody");
        let e = parse(&src).unwrap_err().to_string();
        assert!(e.contains("64"));
    }

    #[test]
    fn rejects_long_compatibility() {
        let long = "a".repeat(501);
        let src = format!("---\nname: t\ndescription: x\ncompatibility: {long}\n---\nbody");
        let e = parse(&src).unwrap_err().to_string();
        assert!(e.contains("500"));
    }

    #[test]
    fn enforces_name_matches_dir() {
        let e = parse_skill_md(
            "---\nname: foo\ndescription: x\n---\nbody",
            Some("bar"),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("parent directory"));
    }

    #[test]
    fn rejects_no_frontmatter() {
        let e = parse("hello").unwrap_err().to_string();
        assert!(e.contains("frontmatter"));
    }

    #[test]
    fn rejects_unclosed_frontmatter() {
        let e = parse("---\nname: t").unwrap_err().to_string();
        assert!(e.contains("closing"));
    }
}
