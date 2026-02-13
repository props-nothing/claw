use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn, debug};

use crate::definition::SkillDefinition;

/// The skill registry — discovers and manages SKILL.md definitions.
///
/// Skills are discovered as directories containing a SKILL.md file.
/// The registry supports three layers with precedence:
/// 1. Workspace skills (project-local, highest priority)
/// 2. User skills (~/.claw/skills/, user-managed)
/// 3. Bundled skills (shipped with the binary, lowest priority)
pub struct SkillRegistry {
    skills: HashMap<String, SkillDefinition>,
    skills_dirs: Vec<PathBuf>,
}

impl SkillRegistry {
    /// Create a new registry with the given skill directories.
    /// Directories are listed in precedence order (first = highest priority).
    pub fn new(dirs: &[&Path]) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dirs: dirs.iter().map(|d| d.to_path_buf()).collect(),
        }
    }

    /// Create a registry with a single skills directory (backwards compat).
    pub fn new_single(skills_dir: &Path) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dirs: vec![skills_dir.to_path_buf()],
        }
    }

    /// Create an empty registry (for tests).
    pub fn new_empty() -> Self {
        Self {
            skills: HashMap::new(),
            skills_dirs: vec![PathBuf::from("/tmp/claw-test-skills")],
        }
    }

    /// Discover and load all SKILL.md definitions from all skill directories.
    /// Later directories have lower precedence (won't override earlier ones).
    pub fn discover(&mut self) -> claw_core::Result<Vec<String>> {
        let mut loaded = Vec::new();

        for dir in self.skills_dirs.clone() {
            if !dir.exists() {
                debug!(?dir, "skills directory does not exist, skipping");
                continue;
            }

            let entries = std::fs::read_dir(&dir).map_err(|e| {
                claw_core::ClawError::Agent(format!("failed to read skills dir {}: {}", dir.display(), e))
            })?;

            for entry in entries {
                let entry = entry.map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;
                let path = entry.path();

                if path.is_dir() {
                    // Look for SKILL.md inside the directory
                    let skill_md = path.join("SKILL.md");
                    if skill_md.exists() {
                        match SkillDefinition::from_file(&skill_md) {
                            Ok(def) => {
                                // Only insert if not already loaded (precedence)
                                if !self.skills.contains_key(&def.name) {
                                    info!(skill = %def.name, path = ?skill_md, "loaded skill");
                                    loaded.push(def.name.clone());
                                    self.skills.insert(def.name.clone(), def);
                                } else {
                                    debug!(
                                        skill = %def.name,
                                        path = ?skill_md,
                                        "skill already loaded from higher-priority directory, skipping"
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(path = ?skill_md, error = %e, "failed to load skill");
                            }
                        }
                    }
                } else if path.extension().is_some_and(|e| e == "md")
                    && path.file_name().is_some_and(|n| n == "SKILL.md")
                {
                    // SKILL.md directly in skills dir (no subdirectory)
                    match SkillDefinition::from_file(&path) {
                        Ok(def) => {
                            if !self.skills.contains_key(&def.name) {
                                info!(skill = %def.name, path = ?path, "loaded skill");
                                loaded.push(def.name.clone());
                                self.skills.insert(def.name.clone(), def);
                            }
                        }
                        Err(e) => {
                            warn!(path = ?path, error = %e, "failed to load skill");
                        }
                    }
                }
            }
        }

        Ok(loaded)
    }

    /// Register a skill definition programmatically.
    pub fn register(&mut self, def: SkillDefinition) {
        let name = def.name.clone();
        self.skills.insert(name, def);
    }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&SkillDefinition> {
        self.skills.get(name)
    }

    /// List all registered skills.
    pub fn list(&self) -> Vec<&SkillDefinition> {
        let mut skills: Vec<_> = self.skills.values().collect();
        skills.sort_by_key(|s| &s.name);
        skills
    }

    /// Remove a skill by name (from registry only, not from disk).
    pub fn remove(&mut self, name: &str) -> bool {
        self.skills.remove(name).is_some()
    }

    /// Get the number of loaded skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Check if any skills are loaded.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Generate the `<available_skills>` block for the system prompt.
    /// Only includes name + description — the LLM reads the full SKILL.md
    /// via file_read when it wants to use a skill.
    pub fn system_prompt_block(&self) -> Option<String> {
        if self.skills.is_empty() {
            return None;
        }

        let mut block = String::from("\n\n<available_skills>\n");
        let mut skills: Vec<_> = self.skills.values().collect();
        skills.sort_by_key(|s| &s.name);

        for skill in &skills {
            block.push_str(&format!(
                "<skill>\n  <name>{}</name>\n  <description>{}</description>\n  <file>{}</file>\n</skill>\n",
                skill.name,
                skill.description,
                skill.file_path.display(),
            ));
        }

        block.push_str(
            "To use a skill: read its SKILL.md file with file_read, then follow the instructions using your tools.\n"
        );
        block.push_str("</available_skills>");

        Some(block)
    }

    /// Get the primary skills directory (first in list).
    pub fn skills_dir(&self) -> &Path {
        self.skills_dirs.first().map(|p| p.as_path()).unwrap_or(Path::new("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_list_skills() {
        let mut reg = SkillRegistry::new_empty();
        let def = SkillDefinition {
            name: "test".into(),
            description: "A test skill".into(),
            version: "1.0.0".into(),
            tags: vec![],
            author: None,
            body: "# Test\nDo stuff.".into(),
            file_path: PathBuf::from("/skills/test/SKILL.md"),
            base_dir: PathBuf::from("/skills/test"),
        };
        reg.register(def);

        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("test").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.count(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    fn remove_skill() {
        let mut reg = SkillRegistry::new_empty();
        reg.register(SkillDefinition {
            name: "removable".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            tags: vec![],
            author: None,
            body: "Body.".into(),
            file_path: PathBuf::new(),
            base_dir: PathBuf::new(),
        });
        assert!(reg.remove("removable"));
        assert!(!reg.remove("removable"));
        assert!(reg.is_empty());
    }

    #[test]
    fn system_prompt_block_format() {
        let mut reg = SkillRegistry::new_empty();
        reg.register(SkillDefinition {
            name: "github".into(),
            description: "Manage GitHub repos and PRs".into(),
            version: "1.0.0".into(),
            tags: vec!["git".into()],
            author: None,
            body: "Instructions here.".into(),
            file_path: PathBuf::from("/skills/github/SKILL.md"),
            base_dir: PathBuf::from("/skills/github"),
        });
        reg.register(SkillDefinition {
            name: "docker".into(),
            description: "Manage Docker containers".into(),
            version: "1.0.0".into(),
            tags: vec![],
            author: None,
            body: "Docker instructions.".into(),
            file_path: PathBuf::from("/skills/docker/SKILL.md"),
            base_dir: PathBuf::from("/skills/docker"),
        });

        let block = reg.system_prompt_block().unwrap();
        assert!(block.contains("<available_skills>"));
        assert!(block.contains("</available_skills>"));
        assert!(block.contains("<name>github</name>"));
        assert!(block.contains("<name>docker</name>"));
        assert!(block.contains("<description>Manage GitHub repos and PRs</description>"));
        assert!(block.contains("<file>/skills/github/SKILL.md</file>"));
        assert!(block.contains("file_read"));
    }

    #[test]
    fn system_prompt_empty_when_no_skills() {
        let reg = SkillRegistry::new_empty();
        assert!(reg.system_prompt_block().is_none());
    }

    #[test]
    fn discover_from_dir() {
        let dir = tempfile::tempdir().unwrap();

        // Create skill directories
        let skill1_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill1_dir).unwrap();
        std::fs::write(
            skill1_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: First skill\n---\n\n# My Skill\n\nDo things.",
        )
        .unwrap();

        let skill2_dir = dir.path().join("another");
        std::fs::create_dir_all(&skill2_dir).unwrap();
        std::fs::write(
            skill2_dir.join("SKILL.md"),
            "---\nname: another\ndescription: Second skill\n---\n\n# Another\n\nMore things.",
        )
        .unwrap();

        // Non-skill directory (no SKILL.md) should be ignored
        let noise_dir = dir.path().join("not-a-skill");
        std::fs::create_dir_all(&noise_dir).unwrap();
        std::fs::write(noise_dir.join("README.md"), "Just a readme.").unwrap();

        let mut reg = SkillRegistry::new_single(dir.path());
        let loaded = reg.discover().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(reg.count(), 2);
        assert!(reg.get("my-skill").is_some());
        assert!(reg.get("another").is_some());
        assert!(reg.get("not-a-skill").is_none());
    }

    #[test]
    fn precedence_higher_dir_wins() {
        let high = tempfile::tempdir().unwrap();
        let low = tempfile::tempdir().unwrap();

        // Same skill name in both dirs
        let high_skill = high.path().join("dup");
        std::fs::create_dir_all(&high_skill).unwrap();
        std::fs::write(
            high_skill.join("SKILL.md"),
            "---\nname: dup\ndescription: High priority version\n---\n\nHigh body.",
        )
        .unwrap();

        let low_skill = low.path().join("dup");
        std::fs::create_dir_all(&low_skill).unwrap();
        std::fs::write(
            low_skill.join("SKILL.md"),
            "---\nname: dup\ndescription: Low priority version\n---\n\nLow body.",
        )
        .unwrap();

        let mut reg = SkillRegistry::new(&[high.path(), low.path()]);
        reg.discover().unwrap();

        let skill = reg.get("dup").unwrap();
        assert_eq!(skill.description, "High priority version");
    }

    #[test]
    fn nonexistent_dir_is_fine() {
        let mut reg = SkillRegistry::new_single(Path::new("/nonexistent/path/to/skills"));
        let loaded = reg.discover().unwrap();
        assert!(loaded.is_empty());
    }
}
