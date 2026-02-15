use super::MeshAction;

pub(super) async fn cmd_mesh(
    config: claw_config::ClawConfig,
    action: MeshAction,
) -> claw_core::Result<()> {
    let listen = &config.server.listen;
    let client = reqwest::Client::builder()
        .tcp_keepalive(None)
        .build()
        .unwrap_or_default();

    let build_req = |url: &str| -> reqwest::RequestBuilder {
        let mut req = client.get(url);
        if let Some(ref key) = config.server.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req
    };

    match action {
        MeshAction::Status => {
            let url = format!("http://{listen}/api/v1/mesh/status");
            let resp = build_req(&url).send().await.map_err(|e| {
                claw_core::ClawError::Agent(format!(
                    "Cannot reach agent at {listen} — is it running? ({e})"
                ))
            })?;

            if !resp.status().is_success() {
                return Err(claw_core::ClawError::Agent(format!(
                    "Server returned {}",
                    resp.status()
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

            println!("🕸️  Mesh Status\n");
            println!(
                "   Enabled:      {}",
                data["enabled"].as_bool().unwrap_or(false)
            );
            println!(
                "   Running:      {}",
                data["running"].as_bool().unwrap_or(false)
            );
            println!(
                "   Peer ID:      {}",
                data["peer_id"].as_str().unwrap_or("—")
            );
            println!(
                "   Peers:        {}",
                data["peer_count"].as_u64().unwrap_or(0)
            );
            println!(
                "   Listen:       {}",
                data["listen"].as_str().unwrap_or("—")
            );
            println!(
                "   mDNS:         {}",
                data["mdns"].as_bool().unwrap_or(false)
            );
            if let Some(caps) = data["capabilities"].as_array() {
                let cap_strs: Vec<&str> = caps.iter().filter_map(|c| c.as_str()).collect();
                println!(
                    "   Capabilities: {}",
                    if cap_strs.is_empty() {
                        "none".to_string()
                    } else {
                        cap_strs.join(", ")
                    }
                );
            }
        }
        MeshAction::Peers => {
            let url = format!("http://{listen}/api/v1/mesh/peers");
            let resp = build_req(&url).send().await.map_err(|e| {
                claw_core::ClawError::Agent(format!(
                    "Cannot reach agent at {listen} — is it running? ({e})"
                ))
            })?;

            if !resp.status().is_success() {
                return Err(claw_core::ClawError::Agent(format!(
                    "Server returned {}",
                    resp.status()
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| claw_core::ClawError::Agent(e.to_string()))?;

            let peers = data["peers"].as_array();
            let count = data["count"].as_u64().unwrap_or(0);

            if count == 0 {
                println!("🕸️  No peers connected.");
                if !config.mesh.enabled {
                    println!("\n   Mesh networking is disabled. Enable it in claw.toml:");
                    println!("   [mesh]");
                    println!("   enabled = true");
                }
            } else {
                println!("🕸️  Mesh Peers ({count} connected)\n");
                if let Some(peers) = peers {
                    for peer in peers {
                        let id = peer["peer_id"].as_str().unwrap_or("?");
                        let host = peer["hostname"].as_str().unwrap_or("?");
                        let os = peer["os"].as_str().unwrap_or("?");
                        let caps: Vec<&str> = peer["capabilities"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();
                        println!("   📡 {id}");
                        println!("      Host: {host} ({os})");
                        println!(
                            "      Capabilities: {}",
                            if caps.is_empty() {
                                "none".to_string()
                            } else {
                                caps.join(", ")
                            }
                        );
                        println!();
                    }
                }
            }
        }
        MeshAction::Send { peer_id, message } => {
            let url = format!("http://{listen}/api/v1/mesh/send");
            let body = serde_json::json!({
                "peer_id": peer_id,
                "message": message,
            });

            let mut req = client.post(&url).json(&body);
            if let Some(ref key) = config.server.api_key {
                req = req.header("Authorization", format!("Bearer {key}"));
            }

            let resp = req.send().await.map_err(|e| {
                claw_core::ClawError::Agent(format!(
                    "Cannot reach agent at {listen} — is it running? ({e})"
                ))
            })?;

            if resp.status().is_success() {
                println!("✅ Message sent to peer {peer_id}");
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(claw_core::ClawError::Agent(format!(
                    "Failed to send message: {status} — {body}"
                )));
            }
        }
    }
    Ok(())
}
