// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Seed the global MCP Servers catalog (`db_mcp_servers`, per-channel) with a
//! curated, credential-free starter set on fresh install.
//!
//! Runs exactly once per channel, tracked in that channel's `db_migrations`
//! table — NOT gated on "is the catalog currently empty," for the same
//! reason `m0015_seed_starter_skills` isn't: that gate can't distinguish
//! "never seeded" from "seeded then the user deleted every starter server on
//! purpose," which would silently resurrect the defaults on the next restart
//! after a full deletion. The migration framework's once-ever-per-channel
//! tracking fixes this at the root: once `0016_seed_starter_mcp_servers` is
//! marked applied, it never runs again for that channel regardless of what
//! the user does to the catalog afterward.
//!
//! `up()` checks for an existing name collision first (same self-heal as
//! m0015) and treats it as "already seeded" rather than inserting — guards
//! against any future retired startup-seed path leaving the six starter
//! names present before this migration ever ran.
//!
//! See docs/specs/SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md
//! for why only credential-free, fully local servers are in this manifest.

use std::sync::Arc;

use crate::backend::mcp_seed::{any_starter_mcp_server_name_exists, seed_starter_mcp_servers};
use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0016SeedStarterMcpServers;

impl Migration for M0016SeedStarterMcpServers {
    fn id(&self) -> &'static str { "0016_seed_starter_mcp_servers" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Seed the global MCP Servers catalog with a curated starter set" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("seed_starter_mcp_servers: open wstore: {}", e)))?,
        );
        if any_starter_mcp_server_name_exists(&wstore)
            .map_err(|e| MigrationError(format!("seed_starter_mcp_servers: check existing: {}", e)))?
        {
            return Ok(());
        }
        seed_starter_mcp_servers(&wstore)
            .map(|_| ())
            .map_err(|e| MigrationError(format!("seed_starter_mcp_servers: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-store.db"),
            channel_store_path: path.to_path_buf(),
        }
    }

    #[test]
    fn seeds_six_starter_mcp_servers_on_a_fresh_channel() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Create the channel store up front — `up()` no-ops when the path
        // doesn't exist yet (mirrors m0007's guard).
        Store::open(tmp.path()).unwrap();

        M0016SeedStarterMcpServers.up(&ctx_for(tmp.path())).unwrap();

        let wstore = Store::open(tmp.path()).unwrap();
        assert_eq!(wstore.mcp_server_list_global().unwrap().len(), 6);
    }

    #[test]
    fn once_marked_applied_a_full_catalog_deletion_is_never_resurrected() {
        // This is the actual fix: the migration framework's db_migrations
        // tracking (not mcp_seed's own logic) is what must prevent
        // reseeding — reproduce that gate here rather than trusting it.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let wstore = Store::open(tmp.path()).unwrap();
        let ctx = ctx_for(tmp.path());

        M0016SeedStarterMcpServers.up(&ctx).unwrap();
        wstore.migration_mark_applied("0016_seed_starter_mcp_servers", "channel", 0).unwrap();
        assert_eq!(wstore.mcp_server_list_global().unwrap().len(), 6);

        for item in wstore.mcp_server_list_global().unwrap() {
            wstore.mcp_server_delete(&item.server.id).unwrap();
        }
        assert!(wstore.mcp_server_list_global().unwrap().is_empty());

        // The real runner (runner.rs) never calls `up()` again once
        // `migration_is_applied` is true — assert that precondition holds,
        // matching the actual gate every real boot goes through.
        assert!(
            wstore.migration_is_applied("0016_seed_starter_mcp_servers"),
            "once applied, the tracking row must persist regardless of catalog contents"
        );
    }

    #[test]
    fn self_heals_when_starter_servers_already_exist_from_a_prior_run() {
        // Mirrors m0015's equivalent test: guards against any future
        // retired startup-seed path leaving starter-server names present
        // before this migration ever ran. Simulate that pre-existing state
        // directly and confirm `up()` succeeds as a no-op instead of
        // erroring on the name-uniqueness collision.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let wstore = Store::open(tmp.path()).unwrap();
        let ctx = ctx_for(tmp.path());

        let pre_existing = crate::backend::storage::mcp_servers::McpServer {
            id: "pre-existing-from-old-path".to_string(),
            name: "git".to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        };
        wstore.mcp_server_upsert_unique_global(&pre_existing).unwrap();

        M0016SeedStarterMcpServers.up(&ctx).unwrap();

        let after = wstore.mcp_server_list_global().unwrap();
        assert_eq!(
            after.len(),
            1,
            "up() must skip seeding entirely on a name collision, not insert the other 5"
        );
        assert_eq!(after[0].server.id, "pre-existing-from-old-path");
    }
}
