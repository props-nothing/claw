use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A skill definition parsed from a SKILL.md file.
///
/// Skills are Markdown documents with YAML frontmatter that contain
/// instructions for the LLM. The runtime does NOT execute skills —
/// the LLM reads the instructions and uses its built-in tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    /// Skill name (from frontmatter or directory name).
    pub name: String,
    /// Short description shown in the system prompt.
    pub description: String,
    /// Semantic version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Author.
    #[serde(default)]
    pub author: Option<String>,
    /// The full Markdown body (instructions for the LLM).
    #[serde(skip)]
    pub body: String,
    /// Absolute path to the SKILL.md file.
    #[serde(skip)]
    pub file_path: PathBuf,
    /// Base directory of the skill (parent of SKILL.md).
    #[serde(skip)]
    pub base_dir: PathBuf,
}

fn default_version() -> String {
    "1.0.0".into()
}

impl SkillDefinition {
    /// Parse a SKILL.md file. The file format is:
    ///
    /// ```text
    /// ---
    /// name: my-skill
    /// description: What this skill does
    /// tags: [tag1, tag2]
    /// ---
    ///
    /// # Skill Title
    ///
    /// Instructions for the LLM...
    /// ```
    pub fn from_file(path: &Path) -> claw_core::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            claw_core::ClawError::Agent(format!("failed to read {}: {}", path.display(), e))
        })?;

        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let file_path = path.to_path_buf();

        Self::parse(&content, file_path, base_dir)
    }

    /// Parse SKILL.md content with known path info.
    pub fn parse(content: &str, file_path: PathBuf, base_dir: PathBuf) -> claw_core::Result<Self> {
        let (frontmatter, body) = split_frontmatter(content)?;

        // Parse YAML frontmatter manually (simple key: value parsing)
        let mut def = parse_frontmatter(&frontmatter, &base_dir)?;
        def.body = body;
        def.file_path = file_path;
        def.base_dir = base_dir;

        // Resolve {baseDir} in body
        let base_dir_str = def.base_dir.to_string_lossy().to_string();
        def.body = def.body.replace("{baseDir}", &base_dir_str);

        if def.name.is_empty() {
            return Err(claw_core::ClawError::Agent("skill name is empty".into()));
        }
        if def.description.is_empty() {
            return Err(claw_core::ClawError::Agent(
                format!("skill '{}' has no description", def.name),
            ));
        }

        Ok(def)
    }

    /// Get the full instructions (body) for injection into conversation context.
    pub fn instructions(&self) -> &str {
        &self.body
    }
}

/// Split a SKILL.md file into YAML frontmatter and Markdown body.
fn split_frontmatter(content: &str) -> claw_core::Result<(String, String)> {
    let trimmed = content.trim();

    // Must start with ---
    if !trimmed.starts_with("---") {
        // No frontmatter — use the whole content as body, but we need at minimum a name
        return Err(claw_core::ClawError::Agent(
            "SKILL.md must start with YAML frontmatter (---)".into(),
        ));
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let end_pos = after_first.find("\n---").ok_or_else(|| {
        claw_core::ClawError::Agent("SKILL.md: missing closing --- for frontmatter".into())
    })?;

    let frontmatter = after_first[..end_pos].trim().to_string();
    let body = after_first[end_pos + 4..].trim().to_string();

    Ok((frontmatter, body))
}

/// Parse simple YAML frontmatter into a SkillDefinition.
/// Supports: name, description, version, tags, author
fn parse_frontmatter(yaml: &str, base_dir: &Path) -> claw_core::Result<SkillDefinition> {
    let mut name = String::new();
    let mut description = String::new();
    let mut version = default_version();
    let mut tags: Vec<String> = Vec::new();
    let mut author: Option<String> = None;

    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "name" => name = unquote(value),
                "description" => description = unquote(value),
                "version" => version = unquote(value),
                "author" => author = Some(unquote(value)),
                "tags" => {
                    // Parse [tag1, tag2] or tag1, tag2
                    let inner = value.trim_start_matches('[').trim_end_matches(']');
                    tags = inner
                        .split(',')
                        .map(|t| unquote(t.trim()))
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                _ => {} // ignore unknown keys
            }
        }
    }

    Ok(SkillDefinition {
        name,
        description,
        version,
        tags,
        author,
        body: String::new(),
        file_path: PathBuf::new(),
        base_dir: base_dir.to_path_buf(),
    })
}

/// Remove surrounding quotes from a YAML value.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_md() {
        let content = r#"---
name: test-skill
description: A test skill for unit testing
version: 2.0.0
tags: [testing, demo]
author: Claw Team
---

# Test Skill

## When to use
When testing the skill parser.

## Instructions
1. Do step one
2. Do step two
"#;
        let def = SkillDefinition::parse(
            content,
            PathBuf::from("/skills/test-skill/SKILL.md"),
            PathBuf::from("/skills/test-skill"),
        )
        .unwrap();

        assert_eq!(def.name, "test-skill");
        assert_eq!(def.description, "A test skill for unit testing");
        assert_eq!(def.version, "2.0.0");
        assert_eq!(def.tags, vec!["testing", "demo"]);
        assert_eq!(def.author, Some("Claw Team".into()));
        assert!(def.body.contains("# Test Skill"));
        assert!(def.body.contains("Do step one"));
    }

    #[test]
    fn parse_minimal_skill() {
        let content = "---\nname: minimal\ndescription: A minimal skill\n---\n\nJust do it.";
        let def = SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .unwrap();

        assert_eq!(def.name, "minimal");
        assert_eq!(def.version, "1.0.0"); // default
        assert_eq!(def.body, "Just do it.");
    }

    #[test]
    fn parse_with_base_dir_replacement() {
        let content = "---\nname: templates\ndescription: Has base dir\n---\n\nRead {baseDir}/data.json";
        let def = SkillDefinition::parse(
            content,
            PathBuf::from("/skills/templates/SKILL.md"),
            PathBuf::from("/skills/templates"),
        )
        .unwrap();

        assert!(def.body.contains("/skills/templates/data.json"));
        assert!(!def.body.contains("{baseDir}"));
    }

    #[test]
    fn missing_frontmatter_errors() {
        let content = "# No frontmatter\nJust markdown.";
        assert!(SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .is_err());
    }

    #[test]
    fn missing_name_errors() {
        let content = "---\ndescription: No name\n---\nBody.";
        assert!(SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .is_err());
    }

    #[test]
    fn missing_description_errors() {
        let content = "---\nname: no-desc\n---\nBody.";
        assert!(SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .is_err());
    }

    #[test]
    fn quoted_values_parsed() {
        let content = "---\nname: \"quoted-skill\"\ndescription: 'Single quoted'\n---\n\nBody.";
        let def = SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .unwrap();

        assert_eq!(def.name, "quoted-skill");
        assert_eq!(def.description, "Single quoted");
    }

    #[test]
    fn tags_parsing_variants() {
        // With brackets
        let content = "---\nname: t1\ndescription: d\ntags: [a, b, c]\n---\n\nBody.";
        let def = SkillDefinition::parse(
            content,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .unwrap();
        assert_eq!(def.tags, vec!["a", "b", "c"]);

        // Without brackets
        let content2 = "---\nname: t2\ndescription: d\ntags: x, y\n---\n\nBody.";
        let def2 = SkillDefinition::parse(
            content2,
            PathBuf::from("/tmp/SKILL.md"),
            PathBuf::from("/tmp"),
        )
        .unwrap();
        assert_eq!(def2.tags, vec!["x", "y"]);
    }

    #[test]
    fn from_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: my-skill\ndescription: From file test\n---\n\n# My Skill\n\nInstructions here.",
        )
        .unwrap();

        let def = SkillDefinition::from_file(&skill_path).unwrap();
        assert_eq!(def.name, "my-skill");
        assert_eq!(def.description, "From file test");
        assert!(def.body.contains("# My Skill"));
        assert_eq!(def.base_dir, skill_dir);
    }
}
