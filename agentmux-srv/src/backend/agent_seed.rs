// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent seed engine: preloads agents from an embedded manifest on first launch.
//! Seeds agents with identity + content. Provider, agent_type, and environment
//! are NOT baked into the manifest — they default to sensible values and are
//! user-configurable via the Agent settings UI after seeding.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use super::storage::memory_bundles::Memory;
use super::storage::store::{AgentDefinition, AgentContent, AgentSkill, Store};
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
    #[serde(default)]
    memories: Vec<SeedMemory>,
}

/// A memory bundle in the seed manifest.
#[derive(Debug, Deserialize)]
struct SeedMemory {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    /// When true this bundle is injected into every agent's CLAUDE.md at
    /// launch (Armory global tier). When false it is available in the
    /// Memory manager but must be selected per-agent.
    #[serde(default)]
    is_global: bool,
    #[serde(default)]
    instructions: String,
}

/// An agent definition in the seed manifest.
#[derive(Debug, Deserialize)]
struct SeedAgent {
    id: String,
    name: String,
    #[serde(default = "default_icon")]
    icon: String,
    /// Defaults to "claude" when absent. User can change in Agent settings UI.
    #[serde(default = "default_provider")]
    provider: String,
    /// Defaults to "host" when absent. User can change in Agent settings UI.
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
    /// Container image to pull and run when agent_type == "container".
    /// Omit for host-only providers. Matches cli-catalog.ts `containerImage`.
    #[serde(default)]
    container_image: String,
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
const SEED_MANIFEST: &str = include_str!("../../agent-seed.json");

/// Seed agent definitions from the embedded manifest.
/// Skips agents whose ID already exists in the database.
pub fn seed_agents(wstore: &Arc<Store>) -> Result<SeedReport, StoreError> {
    let manifest: SeedManifest = serde_json::from_str(SEED_MANIFEST)
        .map_err(|e| StoreError::Other(format!("agent seed: parse manifest: {e}")))?;

    let existing = wstore.agent_def_list()?;
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
        // reuse it as the slug. agent_def_insert collision-resolves if
        // needed and mutates the slug field in place.
        let mut agent = AgentDefinition {
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
            updated_at: now,
            // First seeding always lands templates visible. Phase 2
            // user_hidden is set by `agent_def_set_hidden` after the
            // user explicitly hides; new template ids in re-seed are
            // force-reset to 0 below (see `reseed_if_needed`).
            user_hidden: 0,
            container_image: agent_def.container_image.clone(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(), // manifest doesn't declare this yet
            auto_continue_enabled: 0,
            memory_id: String::new(),
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        };
        wstore.agent_def_insert(&mut agent)?;

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
                    wstore.agent_content_set(&AgentContent {
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
            let skill = AgentSkill {
                id: uuid::Uuid::new_v4().to_string(),
                agent_id: agent_def.id.clone(),
                name: skill_def.name.clone(),
                trigger: skill_def.trigger.clone(),
                skill_type: skill_def.skill_type.clone(),
                description: skill_def.description.clone(),
                content: skill_def.content.clone(),
                created_at: now,
            };
            wstore.agent_skill_insert(&skill)?;
        }

        created += 1;
    }

    Ok(SeedReport { created, skipped })
}

/// Seed memory bundles from the manifest. Skips any bundle whose ID already
/// exists — this is a one-time seed, not an upsert on every startup.
fn seed_memories(wstore: &Arc<Store>, manifest: &SeedManifest) -> Result<usize, StoreError> {
    let existing = wstore.bundle_memory_list()?;
    let existing_ids: std::collections::HashSet<String> =
        existing.iter().map(|m| m.id.clone()).collect();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut created = 0usize;
    for (idx, mem_def) in manifest.memories.iter().enumerate() {
        if existing_ids.contains(&mem_def.id) {
            continue;
        }
        let memory = Memory {
            id: mem_def.id.clone(),
            name: mem_def.name.clone(),
            description: mem_def.description.clone(),
            is_blank: false,
            is_global: mem_def.is_global,
            provider: String::new(),
            model: String::new(),
            instructions: mem_def.instructions.clone(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            // Seed initial global-brain order by manifest position so seeded
            // sections start in a deterministic order; users reorder later.
            sort_order: idx as i64,
            created_at: now,
            updated_at: now,
            is_system: false,
        };
        // Use warn-and-skip rather than ? so a user bundle whose name
        // collides with the seeded name (UNIQUE constraint on name) does
        // not abort the remainder of the seed loop.
        match wstore.bundle_memory_upsert(&memory) {
            Ok(()) => { created += 1; }
            Err(e) => {
                tracing::warn!(
                    id = %mem_def.id,
                    name = %mem_def.name,
                    error = %e,
                    "agent seed: skipping memory bundle due to upsert error (name collision?)"
                );
            }
        }
    }

    Ok(created)
}

/// Run auto-seed on startup. Seeds if empty, or re-seeds if manifest version changed.
/// Re-seeding updates existing seeded agents and removes seeded agents not in the manifest.
pub fn auto_seed_on_startup(wstore: &Arc<Store>) {
    let manifest: SeedManifest = match serde_json::from_str(SEED_MANIFEST) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("agent seed: failed to parse seed manifest: {e}");
            return;
        }
    };

    match wstore.agent_def_count() {
        Ok(0) => {
            tracing::info!("agent seed: no agents found, seeding from manifest v{}...", manifest.version);
            match seed_agents(wstore) {
                Ok(report) => {
                    tracing::info!(
                        "agent seed: seeded {} agents ({} skipped)",
                        report.created,
                        report.skipped
                    );
                }
                Err(e) => tracing::error!("agent seed: failed: {e}"),
            }
        }
        Ok(count) => {
            // Check if we need to re-seed (manifest version changed)
            match reseed_if_needed(wstore, &manifest) {
                Ok(Some(report)) => {
                    tracing::info!(
                        "agent seed: re-seeded from manifest v{}: {} created, {} updated, {} removed",
                        manifest.version, report.created, report.updated, report.removed,
                    );
                }
                Ok(None) => {
                    tracing::info!("agent seed: {} agents exist, manifest up to date", count);
                }
                Err(e) => tracing::error!("agent seed: re-seed failed: {e}"),
            }
        }
        Err(e) => tracing::error!("agent seed: failed to count agents: {e}"),
    }

    // Seed memory bundles once — skips any bundle whose ID already exists.
    if !manifest.memories.is_empty() {
        match seed_memories(wstore, &manifest) {
            Ok(0) => {}
            Ok(n) => tracing::info!("agent seed: seeded {n} memory bundles"),
            Err(e) => tracing::error!("agent seed: failed to seed memories: {e}"),
        }
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
    wstore: &Arc<Store>,
    manifest: &SeedManifest,
) -> Result<Option<ReseedReport>, StoreError> {
    let existing = wstore.agent_def_list()?;

    // Check if any seeded agent needs updating by comparing providers/descriptions
    let manifest_ids: std::collections::HashSet<&str> =
        manifest.agents.iter().map(|a| a.id.as_str()).collect();
    let existing_map: std::collections::HashMap<&str, &AgentDefinition> =
        existing.iter().map(|a| (a.id.as_str(), a)).collect();

    let mut needs_reseed = false;

    // Check for new agents or changed providers
    for agent_def in &manifest.agents {
        match existing_map.get(agent_def.id.as_str()) {
            None => { needs_reseed = true; break; }
            Some(existing_agent) => {
                // Only compare identity fields, NOT provider/agent_type/environment
                // which the user may have changed via the Agent settings UI.
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
        let mut agent = AgentDefinition {
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
            updated_at: now,
            // Newly-added template ids start visible (overwritten below
            // when the row already exists). Phase 2 of the two-tier
            // picker spec (Q2 Decision Y) requires NEW template ids
            // surface once even if a same-named template was previously
            // hidden — the `else` branch below honours that by lining
            // up against existing ids only.
            user_hidden: 0,
            container_image: agent_def.container_image.clone(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(), // manifest doesn't declare this yet
            auto_continue_enabled: 0,
            memory_id: String::new(),
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        };

        if let Some(existing_agent) = existing_map.get(agent_def.id.as_str()) {
            // Preserve user-modified runtime config — only update identity
            // fields (name, icon, description). Everything the user can
            // change in the Agent settings UI stays as-is.
            agent.provider = existing_agent.provider.clone();
            agent.agent_type = existing_agent.agent_type.clone();
            agent.environment = existing_agent.environment.clone();
            agent.model_vendor_base_url = existing_agent.model_vendor_base_url.clone();
            agent.shell = if existing_agent.shell.is_empty() {
                agent_def.shell.clone()
            } else {
                existing_agent.shell.clone()
            };
            agent.auto_start = existing_agent.auto_start;
            agent.restart_on_crash = existing_agent.restart_on_crash;
            agent.created_at = existing_agent.created_at;
            agent.accounts = existing_agent.accounts.clone();
            // Phase 2: preserve the user's hide preference across a
            // manifest re-sync for templates that already exist on disk
            // (the user may have explicitly hidden this one). The
            // newly-added branch below keeps user_hidden = 0 so a never-
            // before-seen template id always surfaces at least once.
            agent.user_hidden = existing_agent.user_hidden;
            wstore.agent_def_update(&mut agent)?;
            updated += 1;
        } else {
            wstore.agent_def_insert(&mut agent)?;
            created += 1;
        }
    }

    // Remove seeded agents not in manifest (e.g., agent4, agent5)
    for agent in &existing {
        if agent.is_seeded == 1 && !manifest_ids.contains(agent.id.as_str()) {
            wstore.agent_def_delete(&agent.id)?;
            removed += 1;
            tracing::info!("agent seed: removed seeded agent '{}'", agent.id);
        }
    }

    Ok(Some(ReseedReport { created, updated, removed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{AgentDefinition, Store};

    /// Helper to build a manifest in-memory with a fixed set of agents.
    /// `reseed_if_needed` reads `manifest.agents`; we construct the
    /// struct directly so the test doesn't have to round-trip JSON.
    fn manifest_with(ids_and_descriptions: &[(&str, &str)]) -> SeedManifest {
        SeedManifest {
            version: 999,
            agents: ids_and_descriptions
                .iter()
                .map(|(id, desc)| SeedAgent {
                    id: id.to_string(),
                    name: id.to_string(),
                    icon: default_icon(),
                    provider: default_provider(),
                    agent_type: default_agent_type(),
                    environment: default_environment(),
                    description: desc.to_string(),
                    working_directory: String::new(),
                    shell: String::new(),
                    agent_bus_id: String::new(),
                    container_image: String::new(),
                    auto_start: false,
                    restart_on_crash: false,
                    content: SeedContent::default(),
                    skills: Vec::new(),
                })
                .collect(),
            memories: Vec::new(),
        }
    }

    fn insert_tpl(wstore: &Arc<Store>, id: &str, name: &str, hidden: i64) {
        let mut def = AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: name.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "v1 desc".to_string(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: hidden,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        };
        wstore.agent_def_insert(&mut def).unwrap();
    }

    #[test]
    fn reseed_preserves_user_hidden_on_existing_templates() {
        // The user previously hid `tpl-claude`. A description-only
        // manifest change triggers a re-seed; the user's hide preference
        // MUST survive (it's a per-user UI flag, not manifest-managed).
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        insert_tpl(&wstore, "tpl-claude", "Claude", 1);
        // Manifest carries a *different* description so reseed_if_needed
        // sees a change and runs the upsert path.
        let manifest = manifest_with(&[("tpl-claude", "v2 desc")]);

        let report = reseed_if_needed(&wstore, &manifest)
            .expect("reseed succeeds")
            .expect("reseed runs because description changed");
        assert_eq!(report.created, 0);
        assert_eq!(report.updated, 1);

        let after = wstore.agent_def_list().unwrap();
        let tpl = after.iter().find(|a| a.id == "tpl-claude").unwrap();
        assert_eq!(tpl.user_hidden, 1, "hide preference must survive reseed");
        assert_eq!(tpl.description, "v2 desc", "description must update");
    }

    #[test]
    fn reseed_resets_user_hidden_on_newly_added_template_id() {
        // The user previously hid `tpl-claude`. A manifest update
        // introduces a brand-new id `tpl-codex`. The new id MUST land
        // with user_hidden = 0 — Phase 2 spec invariant so users always
        // see new templates at least once.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        insert_tpl(&wstore, "tpl-claude", "Claude", 1);
        let manifest = manifest_with(&[
            ("tpl-claude", "v1 desc"), // unchanged — won't fire upsert on its own
            ("tpl-codex", "Codex CLI"),  // NEW id — forces reseed
        ]);

        let report = reseed_if_needed(&wstore, &manifest)
            .expect("reseed succeeds")
            .expect("reseed runs because tpl-codex is new");
        assert!(report.created >= 1, "tpl-codex should be inserted");

        let after = wstore.agent_def_list().unwrap();
        let codex = after
            .iter()
            .find(|a| a.id == "tpl-codex")
            .expect("tpl-codex should now exist");
        assert_eq!(
            codex.user_hidden, 0,
            "newly-added template must start visible (Phase 2 spec invariant)",
        );
        // And the previously-hidden one stays hidden.
        let claude = after.iter().find(|a| a.id == "tpl-claude").unwrap();
        assert_eq!(claude.user_hidden, 1);
    }
}
