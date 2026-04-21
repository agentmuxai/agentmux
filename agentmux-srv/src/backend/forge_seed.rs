// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Forge seed engine: preloads agents from an embedded manifest on first launch.
//! Seeds agents with identity + content. Provider, agent_type, and environment
//! are NOT baked into the manifest — they default to sensible values and are
//! user-configurable via the Forge UI after seeding.

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
    /// Defaults to "claude" when absent. User can change in Forge UI.
    #[serde(default = "default_provider")]
    provider: String,
    /// Defaults to "host" when absent. User can change in Forge UI.
    #[serde(default = "default_agent_type")]
    agent_type: String,
    /// Defaults to the current OS when absent.
    #[serde(default = "default_environment")]
    environment: String,
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

fn default_provider() -> String {
    "claude".to_string()
}

fn default_agent_type() -> String {
    "host".to_string()
}

fn default_environment() -> String {
    std::env::consts::OS.to_string()
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
    #[serde(default)]
    startup: Option<String>,
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

        // Insert agent. For seeded agents, the manifest `id` is already
        // a human-readable slug-form string (agentx, agent1, etc.), so
        // reuse it as the slug. forge_insert collision-resolves if
        // needed and mutates the slug field in place.
        let mut agent = ForgeAgent {
            id: agent_def.id.clone(),
            slug: agent_def.id.clone(),
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
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        wstore.forge_insert(&mut agent)?;

        // Insert content blobs
        let content_pairs = [
            ("agentmd", &agent_def.content.agentmd),
            ("mcp", &agent_def.content.mcp),
            ("env", &agent_def.content.env),
            ("soul", &agent_def.content.soul),
            ("startup", &agent_def.content.startup),
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

/// Run auto-seed on startup. Seeds if empty, or re-seeds if manifest version changed.
/// Re-seeding updates existing seeded agents and removes seeded agents not in the manifest.
pub fn auto_seed_on_startup(wstore: &Arc<WaveStore>) {
    let manifest: SeedManifest = match serde_json::from_str(SEED_MANIFEST) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("forge: failed to parse seed manifest: {e}");
            return;
        }
    };

    match wstore.forge_count() {
        Ok(0) => {
            tracing::info!("forge: no agents found, seeding from manifest v{}...", manifest.version);
            match seed_forge_agents(wstore) {
                Ok(report) => {
                    tracing::info!(
                        "forge: seeded {} agents ({} skipped)",
                        report.created,
                        report.skipped
                    );
                }
                Err(e) => tracing::error!("forge: seed failed: {e}"),
            }
        }
        Ok(count) => {
            // Check if we need to re-seed (manifest version changed)
            match reseed_if_needed(wstore, &manifest) {
                Ok(Some(report)) => {
                    tracing::info!(
                        "forge: re-seeded from manifest v{}: {} created, {} updated, {} removed",
                        manifest.version, report.created, report.updated, report.removed,
                    );
                }
                Ok(None) => {
                    tracing::info!("forge: {} agents exist, manifest up to date", count);
                }
                Err(e) => tracing::error!("forge: re-seed failed: {e}"),
            }
        }
        Err(e) => tracing::error!("forge: failed to count agents: {e}"),
    }
}

/// Report from a re-seed operation.
pub struct ReseedReport {
    pub created: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Re-seed if the manifest version is newer than what's in the DB.
/// Updates seeded agents, adds new ones, removes seeded agents not in the manifest.
fn reseed_if_needed(
    wstore: &Arc<WaveStore>,
    manifest: &SeedManifest,
) -> Result<Option<ReseedReport>, StoreError> {
    let existing = wstore.forge_list()?;

    // Check if any seeded agent needs updating by comparing providers/descriptions
    let manifest_ids: std::collections::HashSet<&str> =
        manifest.agents.iter().map(|a| a.id.as_str()).collect();
    let existing_map: std::collections::HashMap<&str, &ForgeAgent> =
        existing.iter().map(|a| (a.id.as_str(), a)).collect();

    let mut needs_reseed = false;

    // Check for new agents or changed providers
    for agent_def in &manifest.agents {
        match existing_map.get(agent_def.id.as_str()) {
            None => { needs_reseed = true; break; }
            Some(existing_agent) => {
                // Only compare identity fields, NOT provider/agent_type/environment
                // which the user may have changed via the Forge UI.
                if existing_agent.description != agent_def.description {
                    needs_reseed = true;
                    break;
                }
            }
        }
    }

    // Check for agents to remove (seeded agents not in manifest)
    for agent in &existing {
        if agent.is_seeded == 1 && !manifest_ids.contains(agent.id.as_str()) {
            needs_reseed = true;
            break;
        }
    }

    if !needs_reseed {
        return Ok(None);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut removed = 0usize;

    // Upsert agents from manifest
    for agent_def in &manifest.agents {
        let mut agent = ForgeAgent {
            id: agent_def.id.clone(),
            slug: agent_def.id.clone(),
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
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };

        if let Some(existing_agent) = existing_map.get(agent_def.id.as_str()) {
            // Preserve user-modified runtime config — only update identity
            // fields (name, icon, description). Everything the user can
            // change in the Forge UI stays as-is.
            agent.provider = existing_agent.provider.clone();
            agent.agent_type = existing_agent.agent_type.clone();
            agent.environment = existing_agent.environment.clone();
            agent.shell = if existing_agent.shell.is_empty() {
                agent_def.shell.clone()
            } else {
                existing_agent.shell.clone()
            };
            agent.auto_start = existing_agent.auto_start;
            agent.restart_on_crash = existing_agent.restart_on_crash;
            agent.created_at = existing_agent.created_at;
            agent.accounts = existing_agent.accounts.clone();
            wstore.forge_update(&agent)?;
            updated += 1;
        } else {
            wstore.forge_insert(&mut agent)?;
            created += 1;
        }
    }

    // Remove seeded agents not in manifest (e.g., agent4, agent5)
    for agent in &existing {
        if agent.is_seeded == 1 && !manifest_ids.contains(agent.id.as_str()) {
            wstore.forge_delete(&agent.id)?;
            removed += 1;
            tracing::info!("forge: removed seeded agent '{}'", agent.id);
        }
    }

    Ok(Some(ReseedReport { created, updated, removed }))
}
