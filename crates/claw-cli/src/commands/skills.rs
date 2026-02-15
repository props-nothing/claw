use std::path::Path;

use super::{HubAction, SkillAction};

pub(super) async fn cmd_skill(
    config: claw_config::ClawConfig,
    action: SkillAction,
    config_path: &Path,
) -> claw_core::Result<()> {
    // Resolve skills_dir relative to config directory (e.g. ~/.claw/skills)
    let config_dir = config_path.parent().unwrap_or(Path::new("."));
    let skills_dir = if config.plugins.plugin_dir.is_absolute() {
        config
            .plugins
            .plugin_dir
            .parent()
            .unwrap_or(Path::new("."))
            .join("skills")
    } else {
        config_dir.join("skills")
    };

    let mut registry = claw_skills::SkillRegistry::new_single(&skills_dir);
    let _ = registry.discover();

    match action {
        SkillAction::List => {
            let skills = registry.list();
            if skills.is_empty() {
                println!("No skills found in {}", skills_dir.display());
                println!("  Create one with: claw skill create <name>");
            } else {
                println!("\x1b[1mAvailable Skills ({}):\x1b[0m\n", skills.len());
                for s in skills {
                    let tags = if s.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", s.tags.join(", "))
                    };
                    println!("  \x1b[36m{}\x1b[0m v{}{}", s.name, s.version, tags);
                    println!("    {}", s.description);
                    println!("    File: {}", s.file_path.display());
                    println!();
                }
            }
        }
        SkillAction::Show { name } => match registry.get(&name) {
            Some(skill) => {
                println!("\x1b[1m{}\x1b[0m v{}", skill.name, skill.version);
                println!("  {}", skill.description);
                if let Some(ref author) = skill.author {
                    println!("  Author: {author}");
                }
                if !skill.tags.is_empty() {
                    println!("  Tags: {}", skill.tags.join(", "));
                }
                println!("  File: {}", skill.file_path.display());

                println!("\n  \x1b[1mInstructions:\x1b[0m");
                for line in skill.body.lines() {
                    println!("    {line}");
                }
            }
            None => {
                println!("Skill '{name}' not found.");
            }
        },
        SkillAction::Run { name, param: _ } => match registry.get(&name) {
            Some(skill) => {
                println!("📖 Skill '{name}' — SKILL.md instructions:\n");
                println!("{}", skill.body);
                println!("\n\x1b[33mNote:\x1b[0m Skills are now prompt-injected instructions.");
                println!(
                    "The LLM reads these instructions and uses built-in tools to execute them."
                );
                println!("Start the agent with 'claw start' and ask it to use this skill.");
            }
            None => {
                println!("Skill '{name}' not found.");
            }
        },
        SkillAction::Create { name } => {
            let skill_dir = skills_dir.join(&name);
            if skill_dir.exists() {
                return Err(claw_core::ClawError::Agent(format!(
                    "Skill '{}' already exists at {}",
                    name,
                    skill_dir.display()
                )));
            }

            std::fs::create_dir_all(&skill_dir)?;
            let skill_path = skill_dir.join("SKILL.md");

            let template = format!(
                r#"---
name: {name}
description: Describe what this skill does
version: 1.0.0
tags: []
---

# {name}

## When to use this skill

Describe when this skill should be activated.

## Instructions

1. First, do this using `shell_exec`
2. Then check the result
3. Finally, report back to the user

## Notes

- Add any important notes or caveats here
- Reference files in this skill directory with {{baseDir}}
"#
            );

            std::fs::write(&skill_path, template)?;
            println!("✅ Created skill template at {}", skill_path.display());
            println!(
                "   Edit the SKILL.md, then start the agent — it will discover the skill automatically."
            );
        }
        SkillAction::Delete { name } => {
            let skill_dir = skills_dir.join(&name);
            if skill_dir.exists() {
                std::fs::remove_dir_all(&skill_dir)?;
                println!("✅ Deleted skill directory '{}'", skill_dir.display());
            } else {
                // Try removing from registry
                if registry.remove(&name) {
                    println!("✅ Removed skill '{name}' from registry");
                } else {
                    println!("Skill '{name}' not found.");
                }
            }
        }
        SkillAction::Push { name } => {
            // Resolve hub URL — services.hub_url is required for push/pull/search
            let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                claw_core::ClawError::Agent(
                    "No hub_url configured. Set services.hub_url in claw.toml or run 'claw hub serve' to host your own hub.".into(),
                )
            })?;
            let hub_url = hub_url.trim_end_matches('/');

            // Read the local SKILL.md and push it to the hub
            let skill = registry.get(&name).ok_or_else(|| {
                claw_core::ClawError::Agent(
                    format!("Skill '{name}' not found locally. Use 'claw skill list' to see available skills."),
                )
            })?;

            // Read the raw SKILL.md file
            let skill_content = std::fs::read_to_string(&skill.file_path)?;

            let client = reqwest::Client::builder()
                .tcp_keepalive(None)
                .build()
                .unwrap_or_default();
            let url = format!("{hub_url}/api/v1/hub/skills");

            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "skill_content": skill_content }))
                .send()
                .await
                .map_err(|e| {
                    claw_core::ClawError::Agent(format!(
                        "Cannot reach hub at {hub_url} — is it running? ({e})"
                    ))
                })?;

            if resp.status().is_success() {
                let data: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;
                println!(
                    "✅ Published '{}' v{} to Skills Hub at {}",
                    data["name"].as_str().unwrap_or(&name),
                    data["version"].as_str().unwrap_or("?"),
                    hub_url
                );
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(claw_core::ClawError::Agent(format!(
                    "Hub returned {status}: {body}"
                )));
            }
        }
        SkillAction::Pull { name } => {
            let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                claw_core::ClawError::Agent(
                    "No hub_url configured. Set services.hub_url in claw.toml.".into(),
                )
            })?;
            let hub_url = hub_url.trim_end_matches('/');

            let client = reqwest::Client::builder()
                .tcp_keepalive(None)
                .build()
                .unwrap_or_default();
            let url = format!("{hub_url}/api/v1/hub/skills/{name}/pull");

            let resp = client.post(&url).send().await.map_err(|e| {
                claw_core::ClawError::Agent(format!(
                    "Cannot reach hub at {hub_url} — is it running? ({e})"
                ))
            })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(claw_core::ClawError::Agent(format!(
                    "Hub returned {status}: {body}"
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

            let skill_content = data["skill_content"].as_str().unwrap_or("");
            let version = data["version"].as_str().unwrap_or("?");
            let skill_name = data["name"].as_str().unwrap_or(&name);

            // Save to local skills directory as SKILL.md in a subdirectory
            let skill_dir = skills_dir.join(skill_name);
            std::fs::create_dir_all(&skill_dir)?;
            let path = skill_dir.join("SKILL.md");
            std::fs::write(&path, skill_content)?;
            println!(
                "✅ Pulled '{}' v{} from {} → {}",
                skill_name,
                version,
                hub_url,
                path.display()
            );
        }
        SkillAction::Search { query, tag } => {
            let hub_url = config.services.hub_url.as_deref().ok_or_else(|| {
                claw_core::ClawError::Agent(
                    "No hub_url configured. Set services.hub_url in claw.toml.".into(),
                )
            })?;
            let hub_url = hub_url.trim_end_matches('/');

            let client = reqwest::Client::builder()
                .tcp_keepalive(None)
                .build()
                .unwrap_or_default();
            let mut url = format!("{hub_url}/api/v1/hub/skills/search?q={query}");
            if let Some(ref t) = tag {
                url.push_str(&format!("&tag={t}"));
            }

            let resp = client.get(&url).send().await.map_err(|e| {
                claw_core::ClawError::Agent(format!(
                    "Cannot reach hub at {hub_url} — is it running? ({e})"
                ))
            })?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(claw_core::ClawError::Agent(format!(
                    "Hub returned {status}"
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

            let skills = data["skills"].as_array();
            match skills {
                Some(skills) if !skills.is_empty() => {
                    println!(
                        "🔍 Found {} skill(s) matching '{}' on {}:\n",
                        skills.len(),
                        query,
                        hub_url
                    );
                    for s in skills {
                        let name = s["name"].as_str().unwrap_or("?");
                        let desc = s["description"].as_str().unwrap_or("");
                        let ver = s["version"].as_str().unwrap_or("?");
                        let dl = s["downloads"].as_u64().unwrap_or(0);
                        let tags: Vec<&str> = s["tags"]
                            .as_array()
                            .map(|t| t.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();

                        println!("  📦 {name} v{ver} (⬇ {dl})");
                        if !desc.is_empty() {
                            println!("     {desc}");
                        }
                        if !tags.is_empty() {
                            println!("     tags: {}", tags.join(", "));
                        }
                        println!();
                    }
                    println!("Pull a skill with: claw skill pull <name>");
                }
                _ => {
                    println!("No skills found matching '{query}' on {hub_url}");
                }
            }
        }
    }
    Ok(())
}

pub(super) async fn cmd_hub(action: HubAction) -> claw_core::Result<()> {
    match action {
        HubAction::Serve { listen, db } => {
            let db_path = match db {
                Some(p) => std::path::PathBuf::from(p),
                None => dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".claw")
                    .join("hub.db"),
            };

            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            println!("🦞 Claw Skills Hub");
            println!("   Database: {}", db_path.display());
            println!("   Listening: http://{listen}");
            println!();
            println!("   Remote agents should set in their claw.toml:");
            println!("   [services]");
            println!("   hub_url = \"http://{listen}\"");
            println!();

            let router = claw_server::hub::standalone_hub_router(&db_path)
                .map_err(claw_core::ClawError::Agent)?;

            let listener = tokio::net::TcpListener::bind(&listen).await.map_err(|e| {
                claw_core::ClawError::Agent(format!("Failed to bind {listen}: {e}"))
            })?;

            println!("✅ Hub server started — press Ctrl+C to stop\n");

            axum::serve(listener, router)
                .await
                .map_err(|e| claw_core::ClawError::Agent(format!("Hub server error: {e}")))?;
        }
    }
    Ok(())
}
