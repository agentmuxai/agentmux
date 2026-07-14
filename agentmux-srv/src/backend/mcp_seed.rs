// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Starter MCP Servers catalog seed: preloads a small set of curated global,
//! credential-free MCP servers (`db_mcp_servers`, the v1 standalone catalog)
//! on fresh install.
//!
//! Mirrors `skill_seed.rs` exactly, for the same reasons documented there:
//! the actual "run exactly once, ever" gating lives in
//! `migrations::m0016_seed_starter_mcp_servers`, tracked in the channel's
//! `db_migrations` table — NOT in this module, and NOT gated on "is the
//! catalog currently empty" (that gate can't distinguish "never seeded" from
//! "seeded then the user deleted every starter server on purpose", which
//! would silently resurrect the defaults on the next restart after a full
//! deletion — see `m0015_seed_starter_skills.rs`'s history, reagent P2 on
//! PR #2141 round 2, for why this shape was chosen over a simpler one). This
//! module exposes only the pure insert logic; the migration owns invocation.
//!
//! Only credential-free, fully local servers belong in this manifest — see
//! docs/specs/SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md §2:
//! every `is_global` MCP server is auto-injected into every agent's
//! `.mcp.json` at launch with no per-agent opt-in, so a seeded row needing an
//! API key or OAuth token would break every agent's launch by default. The
//! Filesystem reference server is deliberately NOT included despite being
//! credential-free — it requires an allowlisted directory as a CLI arg with
//! no environment-agnostic default (spec §9 open question 3); it can be
//! added once that's resolved, either by computing a per-agent default at
//! config-build time or shipping it with an explicit prereq the user fills
//! in via the existing catalog-picker mechanism instead.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use uuid::Uuid;

use super::storage::mcp_servers::McpServer;
use super::storage::store::Store;
use super::storage::StoreError;

/// One entry in the embedded starter-mcp-servers manifest. `config` is a raw
/// JSON object here (command/args/env) and gets re-serialized to a string to
/// match `McpServer.config`'s storage shape.
#[derive(Debug, Deserialize)]
struct StarterMcpServer {
    name: String,
    transport: String,
    config: serde_json::Value,
}

/// The embedded starter-mcp-servers manifest JSON. Content is authored
/// externally and must not be edited here — see
/// `agentmux-srv/src/config/starter-mcp-servers.json`.
const STARTER_MCP_SERVERS_JSON: &str = include_str!("../config/starter-mcp-servers.json");

/// Report returned after a seed attempt.
pub struct McpServerSeedReport {
    pub created: usize,
}

/// True if any global MCP server whose name matches a starter entry's name
/// already exists — i.e. this channel already effectively has the starter
/// set (or a name collision with it). The caller
/// (`migrations::m0016_seed_starter_mcp_servers`) uses this to skip seeding
/// entirely rather than attempting an insert that would collide on
/// `mcp_server_upsert_unique_global`'s name-uniqueness check and permanently
/// fail the migration on every subsequent boot — same self-heal
/// `skill_seed.rs::any_starter_skill_name_exists` provides (reagent P1, PR
/// #2144).
pub(crate) fn any_starter_mcp_server_name_exists(wstore: &Arc<Store>) -> Result<bool, StoreError> {
    let manifest: Vec<StarterMcpServer> = serde_json::from_str(STARTER_MCP_SERVERS_JSON)
        .map_err(|e| StoreError::Other(format!("mcp server seed: parse manifest: {e}")))?;
    let existing = wstore.mcp_server_list_global()?;
    Ok(manifest
        .iter()
        .any(|entry| existing.iter().any(|item| item.server.name == entry.name)))
}

/// Parse the embedded manifest and insert every entry as a global MCP server
/// via the validated `mcp_server_upsert_unique_global` path. Does NOT check
/// whether the catalog is already populated — the caller
/// (`migrations::m0016_seed_starter_mcp_servers`) owns run-once gating via
/// `db_migrations` tracking, not catalog contents. Exposed at `pub(crate)`
/// so the migration and tests can call it directly.
///
/// All-or-nothing: `mcp_server_upsert_unique_global` commits each insert in
/// its own transaction, so a mid-loop failure can't be rolled back by the
/// database itself. If any insert fails, this compensates by deleting every
/// server already inserted in THIS call before returning the error —
/// otherwise a retry (the migration framework re-runs `up()` on the next
/// boot when a migration returns `Err`, since it's never marked applied)
/// would hit the name-uniqueness rejection on the servers already stranded
/// from the failed attempt. Mirrors `skill_seed::seed_starter_skills`
/// exactly (reagent P2, PR #2141 round 1).
pub(crate) fn seed_starter_mcp_servers(wstore: &Arc<Store>) -> Result<McpServerSeedReport, StoreError> {
    let manifest: Vec<StarterMcpServer> = serde_json::from_str(STARTER_MCP_SERVERS_JSON)
        .map_err(|e| StoreError::Other(format!("mcp server seed: parse manifest: {e}")))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut inserted_ids: Vec<String> = Vec::with_capacity(manifest.len());
    for entry in &manifest {
        let config = serde_json::to_string(&entry.config)
            .map_err(|e| StoreError::Other(format!("mcp server seed: serialize config for '{}': {e}", entry.name)))?;
        let server = McpServer {
            id: Uuid::new_v4().to_string(),
            name: entry.name.clone(),
            transport: entry.transport.clone(),
            config,
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = wstore.mcp_server_upsert_unique_global(&server) {
            for id in &inserted_ids {
                if let Err(cleanup_err) = wstore.mcp_server_delete(id) {
                    tracing::error!(
                        "mcp server seed: cleanup after partial failure could not remove {id}: {cleanup_err}"
                    );
                }
            }
            return Err(e);
        }
        inserted_ids.push(server.id);
    }

    Ok(McpServerSeedReport { created: inserted_ids.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_six_mcp_servers_into_an_empty_catalog() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        assert!(wstore.mcp_server_list_global().unwrap().is_empty());

        let report = seed_starter_mcp_servers(&wstore).unwrap();

        assert_eq!(report.created, 6);
        let after = wstore.mcp_server_list_global().unwrap();
        assert_eq!(after.len(), 6, "all six starter MCP servers should be seeded");
        assert!(after.iter().all(|item| item.server.is_global));
        assert!(
            after.iter().all(|item| item.server.transport == "stdio"),
            "every Tier A entry is a local stdio process"
        );
    }

    #[test]
    fn seeded_config_round_trips_as_valid_json() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        seed_starter_mcp_servers(&wstore).unwrap();

        let after = wstore.mcp_server_list_global().unwrap();
        let git = after
            .iter()
            .find(|item| item.server.name == "git")
            .expect("git should be seeded");
        let parsed: serde_json::Value = serde_json::from_str(&git.server.config).unwrap();
        assert_eq!(parsed["command"], "uvx");
        assert_eq!(parsed["args"][0], "mcp-server-git");
    }

    #[test]
    fn a_failed_insert_rolls_back_the_ones_already_seeded_this_call() {
        // Same failure mode as skill_seed.rs's equivalent test (reagent P2,
        // PR #2141 round 1): mcp_server_upsert_unique_global commits each
        // insert in its own transaction, so a mid-loop failure can't be
        // rolled back by the database. Simulate it by pre-inserting a global
        // server whose NAME collides with one of the starter entries — this
        // forces seed_starter_mcp_servers to fail partway through the
        // manifest, and the catalog must end up back at exactly the one
        // pre-existing server, not a stranded partial starter set.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // "fetch" is the second entry in the manifest — colliding on it
        // guarantees at least one successful insert ("git") precedes the
        // failure, so the rollback path actually has something to clean up.
        let colliding = McpServer {
            id: "pre-existing-collision".to_string(),
            name: "fetch".to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        wstore.mcp_server_upsert_unique_global(&colliding).unwrap();

        let result = seed_starter_mcp_servers(&wstore);
        assert!(result.is_err(), "seeding must fail when a name collides");

        let after = wstore.mcp_server_list_global().unwrap();
        assert_eq!(
            after.len(),
            1,
            "a failed seed must roll back every server it inserted this call, leaving only the pre-existing one"
        );
        assert_eq!(after[0].server.id, "pre-existing-collision");
    }

    #[test]
    fn any_starter_mcp_server_name_exists_detects_a_collision_without_inserting() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        assert!(!any_starter_mcp_server_name_exists(&wstore).unwrap());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let user_created = McpServer {
            id: "user-server".to_string(),
            name: "memory".to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        wstore.mcp_server_upsert_unique_global(&user_created).unwrap();

        assert!(any_starter_mcp_server_name_exists(&wstore).unwrap());
        assert_eq!(
            wstore.mcp_server_list_global().unwrap().len(),
            1,
            "the check itself must not insert anything"
        );
    }
}
