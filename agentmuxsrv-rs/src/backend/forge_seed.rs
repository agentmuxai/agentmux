// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Forge seed engine: preloads agents from an embedded manifest on first launch.
//! Seeds host agents (AgentX, AgentY) and container agents (Agent1-5) with their
//! full configuration including content blobs (CLAUDE.md, MCP, env) and skills.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use super::storage::wstore::{ForgeAgent, ForgeContent, ForgeSkill, WaveStore};
use super::storage::StoreError;

/// Report returned after seeding.
pub struct SeedReport {
    pub created: usize,
    pub skipped: usize,
}

/// Top-level seed manifest structure.
#[derive(Debug, Deserialize)]
struct SeedManifest {
    #[allow(dead_code)]
    version: u32,
    agents: Vec<SeedAgent>,
}

/// An agent definition in the seed manifest.
#[derive(Debug, Deserialize)]
struct SeedAgent {
    id: String,
    name: String,
    #[serde(default = "default_icon")]
    icon: String,
    agent_type: String,
    environment: String,
    provider: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    working_directory: String,
    #[serde(default)]
    shell: String,
    #[serde(default)]
    agent_bus_id: String,
    #[serde(default)]
    auto_start: bool,
    #[serde(default)]
    restart_on_crash: bool,
    #[serde(default)]
    content: SeedContent,
    #[serde(default)]
    skills: Vec<SeedSkill>,
}

fn default_icon() -> String {
    "\u{2726}".to_string()
}

/// Content blobs to seed for an agent.
#[derive(Debug, Default, Deserialize)]
struct SeedContent {
    #[serde(default)]
    agentmd: Option<String>,
    #[serde(default)]
    mcp: Option<String>,
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    soul: Option<String>,
}

/// A skill definition in the seed manifest.
#[derive(Debug, Deserialize)]
struct SeedSkill {
    name: String,
    #[serde(default)]
    trigger: String,
    #[serde(default = "default_skill_type")]
    skill_type: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    content: String,
}

fn default_skill_type() -> String {
    "prompt".to_string()
}

/// The embedded seed manifest JSON.
const SEED_MANIFEST: &str = include_str!("../../forge-seed.json");

/// Seed forge agents from the embedded manifest.
/// Skips agents whose ID already exists in the database.
pub fn seed_forge_agents(wstore: &Arc<WaveStore>) -> Result<SeedReport, StoreError> {
    let manifest: SeedManifest = serde_json::from_str(SEED_MANIFEST)
        .map_err(|e| StoreError::Other(format!("forge seed: parse manifest: {e}")))?;

    let existing = wstore.forge_list()?;
    let existing_ids: std::collections::HashSet<String> =
        existing.iter().map(|a| a.id.clone()).collect();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut created = 0usize;
    let mut skipped = 0usize;

    for agent_def in &manifest.agents {
        if existing_ids.contains(&agent_def.id) {
            skipped += 1;
            continue;
        }

        // Insert agent
        let agent = ForgeAgent {
            id: agent_def.id.clone(),
            name: agent_def.name.clone(),
            icon: agent_def.icon.clone(),
            provider: agent_def.provider.clone(),
            description: agent_def.description.clone(),
            working_directory: agent_def.working_directory.clone(),
            shell: agent_def.shell.clone(),
            provider_flags: String::new(),
            auto_start: if agent_def.auto_start { 1 } else { 0 },
            restart_on_crash: if agent_def.restart_on_crash { 1 } else { 0 },
            idle_timeout_minutes: 0,
            created_at: now,
            agent_type: agent_def.agent_type.clone(),
            environment: agent_def.environment.clone(),
            agent_bus_id: agent_def.agent_bus_id.clone(),
            is_seeded: 1,
        };
        wstore.forge_insert(&agent)?;

        // Insert content blobs
        let content_pairs = [
            ("agentmd", &agent_def.content.agentmd),
            ("mcp", &agent_def.content.mcp),
            ("env", &agent_def.content.env),
            ("soul", &agent_def.content.soul),
        ];
        for (content_type, maybe_content) in &content_pairs {
            if let Some(content) = maybe_content {
                if !content.is_empty() {
                    wstore.forge_set_content(&ForgeContent {
                        agent_id: agent_def.id.clone(),
                        content_type: content_type.to_string(),
                        content: content.clone(),
                        updated_at: now,
                    })?;
                }
            }
        }

        // Insert skills
        for skill_def in &agent_def.skills {
            let skill = ForgeSkill {
                id: uuid::Uuid::new_v4().to_string(),
                agent_id: agent_def.id.clone(),
                name: skill_def.name.clone(),
                trigger: skill_def.trigger.clone(),
                skill_type: skill_def.skill_type.clone(),
                description: skill_def.description.clone(),
                content: skill_def.content.clone(),
                created_at: now,
            };
            wstore.forge_insert_skill(&skill)?;
        }

        created += 1;
    }

    Ok(SeedReport { created, skipped })
}

/// Run auto-seed on startup: only seeds if the forge agents table is empty.
pub fn auto_seed_on_startup(wstore: &Arc<WaveStore>) {
    match wstore.forge_count() {
        Ok(count) if count == 0 => {
            tracing::info!("forge: no agents found, seeding from manifest...");
            match seed_forge_agents(wstore) {
                Ok(report) => {
                    tracing::info!(
                        "forge: seeded {} agents ({} skipped)",
                        report.created,
                        report.skipped
                    );
                }
                Err(e) => {
                    tracing::error!("forge: seed failed: {e}");
                }
            }
        }
        Ok(count) => {
            tracing::info!("forge: {} agents exist, skipping auto-seed", count);
        }
        Err(e) => {
            tracing::error!("forge: failed to count agents: {e}");
        }
    }
}
