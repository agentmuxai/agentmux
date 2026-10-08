// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bundles stop carrying MCP servers: they belong to Connectors
//! (`docs/specs/SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1, D3).
//!
//! For this channel:
//! - logs each agent that loses a server it reached only through a bundle,
//!   naming the agent and the server, so the loss shows in the srv log;
//! - deletes the servers private to a bundle from the identity store: not
//!   global, referenced by a bundle here, and bound to no agent here. An
//!   agent can't bind another entity's private server (`mcp.catalog.bind`
//!   takes global ones only), so no other channel's agent holds one either;
//! - deletes every `db_bundle_mcp_ref` row;
//! - clears the shared store's inline `db_bundles.mcp_servers` to `[]`.
//!
//! An agent's own servers (`db_agent_mcp_ref`) and the global ones are left
//! alone. `db_bundle_mcp_ref` stays defined, because an older build on the
//! same channel expects it; dropping it is a later cleanup.
//!
//! Idempotent: a second run finds no refs and nothing inline. Fails (to be
//! retried on the next boot) when the shared store can't be opened or the
//! inline column can't be cleared, since those hold server configs. Never
//! fails over the identity store: when it can't be opened, the private
//! servers are left (unreachable, since their refs go) and that is logged.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rusqlite::params;

use crate::backend::storage::store::Store;
use crate::registry;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0036DropBundleMcp;

impl Migration for M0036DropBundleMcp {
    fn id(&self) -> &'static str { "0036_drop_bundle_mcp" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Take MCP servers out of bundles; agents keep their own and the global ones"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drop_bundle_mcp: open mstore: {e}")))?,
        );
        // The shared store itself, not m0021's best-effort fallback to the
        // channel store: clearing that (usually empty) local copy would record
        // this as done with the shared configs still stored. Unavailable means
        // fail, and retry on the next boot.
        let bundle_store = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("drop_bundle_mcp: open shared store: {e}")))?;
        let identity_store = match open_identity_store() {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!(error = %e, "drop_bundle_mcp: identity store unavailable, bundle-private servers left in place");
                None
            }
        };
        drop_bundle_mcp(&mstore, identity_store.as_ref(), &bundle_store).map_err(MigrationError)
    }
}

fn open_identity_store() -> Result<Store, String> {
    let path = registry::resolve_identity_store_path()
        .ok_or_else(|| "could not resolve identity store path".to_string())?;
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    Store::open_identity_store(&path).map_err(|e| format!("open identity store: {e}"))
}

/// What one run removed, for the log and the tests.
#[derive(Debug, Default, PartialEq, Eq)]
struct Dropped {
    refs: usize,
    private_servers: usize,
    inline_bundles: usize,
}

fn drop_bundle_mcp(mstore: &Store, identity_store: Option<&Store>, bundle_store: &Store) -> Result<(), String> {
    let refs = bundle_mcp_refs(mstore).map_err(|e| format!("drop_bundle_mcp: read refs: {e}"))?;
    let mut dropped = Dropped::default();

    if !refs.is_empty() {
        log_agents_losing_servers(mstore, identity_store, &refs);

        if let Some(identity_store) = identity_store {
            let agent_bound = agent_bound_mcp_ids(mstore).map_err(|e| format!("drop_bundle_mcp: read agent refs: {e}"))?;
            let referenced: BTreeSet<&str> = refs.values().flatten().map(String::as_str).collect();
            for id in referenced {
                if agent_bound.contains(id) {
                    continue;
                }
                let deleted = identity_store
                    .conn()
                    .lock()
                    .unwrap()
                    .execute("DELETE FROM db_mcp_servers WHERE id = ?1 AND is_global = 0", params![id])
                    .map_err(|e| format!("drop_bundle_mcp: delete server {id}: {e}"))?;
                dropped.private_servers += deleted;
            }
        }

        dropped.refs = mstore
            .conn()
            .lock()
            .unwrap()
            .execute("DELETE FROM db_bundle_mcp_ref", [])
            .map_err(|e| format!("drop_bundle_mcp: delete refs: {e}"))?;
    }

    // The inline column, which nothing has read since the ref tables became
    // authoritative, but which still holds server configs, credentials
    // included. A failure fails the migration, so it's retried on the next
    // boot rather than recorded as done with the configs still stored.
    dropped.inline_bundles = bundle_store
        .conn()
        .lock()
        .unwrap()
        .execute("UPDATE db_bundles SET mcp_servers = '[]' WHERE mcp_servers NOT IN ('', '[]')", [])
        .map_err(|e| format!("drop_bundle_mcp: clear inline mcp_servers: {e}"))?;

    tracing::info!(
        refs = dropped.refs,
        private_servers = dropped.private_servers,
        inline_bundles = dropped.inline_bundles,
        "drop_bundle_mcp: complete"
    );
    Ok(())
}

/// Every bundle's MCP refs on this channel, bundle id to server ids.
fn bundle_mcp_refs(mstore: &Store) -> rusqlite::Result<BTreeMap<String, Vec<String>>> {
    let conn = mstore.conn().lock().unwrap();
    let mut stmt = conn.prepare("SELECT bundle_id, mcp_id FROM db_bundle_mcp_ref ORDER BY bundle_id, mcp_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let (bundle_id, mcp_id) = row?;
        out.entry(bundle_id).or_default().push(mcp_id);
    }
    Ok(out)
}

fn agent_bound_mcp_ids(mstore: &Store) -> rusqlite::Result<BTreeSet<String>> {
    let conn = mstore.conn().lock().unwrap();
    let mut stmt = conn.prepare("SELECT DISTINCT mcp_id FROM db_agent_mcp_ref")?;
    let ids = stmt.query_map([], |r| r.get::<_, String>(0))?;
    ids.collect()
}

/// One log line per agent that reached a server only through its bundles: not
/// a global one, and not one bound to the agent itself.
fn log_agents_losing_servers(
    mstore: &Store,
    identity_store: Option<&Store>,
    refs: &BTreeMap<String, Vec<String>>,
) {
    let Ok(defs) = mstore.agent_def_list() else {
        return;
    };
    let server = |id: &str| identity_store.and_then(|s| s.mcp_server_get(id).ok().flatten());
    for def in defs {
        let own: BTreeSet<String> = {
            let conn = mstore.conn().lock().unwrap();
            let mut stmt = match conn.prepare("SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?1") {
                Ok(stmt) => stmt,
                Err(_) => continue,
            };
            let ids = stmt.query_map(params![def.id], |r| r.get::<_, String>(0));
            match ids {
                Ok(ids) => ids.flatten().collect(),
                Err(_) => continue,
            }
        };
        let mut lost: BTreeSet<String> = BTreeSet::new();
        for bundle_id in mstore.agent_bundle_chain(&def.id) {
            for id in refs.get(&bundle_id).into_iter().flatten() {
                if own.contains(id) {
                    continue;
                }
                match server(id) {
                    Some(s) if s.is_global => {}
                    Some(s) => {
                        lost.insert(s.name);
                    }
                    // Unknown without the identity store; name it by id.
                    None => {
                        lost.insert(id.clone());
                    }
                }
            }
        }
        if !lost.is_empty() {
            tracing::warn!(
                agent_id = %def.id,
                agent = %def.name,
                servers = %lost.into_iter().collect::<Vec<_>>().join(", "),
                "drop_bundle_mcp: agent no longer gets these MCP servers from its bundles; bind them in Connectors"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;
    use crate::backend::storage::store::Bundle;
    use crate::backend::storage::McpServer;

    fn bundle(id: &str, inline_mcp: &str) -> Bundle {
        Bundle {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: inline_mcp.to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        }
    }

    fn server(id: &str, is_global: bool) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: format!("name-{id}"),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn exec(store: &Store, sql: &str, args: &[&str]) {
        store.conn().lock().unwrap().execute(sql, rusqlite::params_from_iter(args)).unwrap();
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.conn().lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn bundle_refs_and_bundle_private_servers_go_and_everything_else_stays() {
        let mstore = Store::open_in_memory().unwrap();
        let identity = Store::open_in_memory().unwrap();
        let shared = Store::open_in_memory().unwrap();

        shared.bundle_upsert(&bundle("b1", "[]")).unwrap();
        // What an older build left inline; bundle_upsert itself always stores "[]".
        exec(&shared, r#"UPDATE db_bundles SET mcp_servers = '[{"name":"inline","env":{"T":"x"}}]' WHERE id = 'b1'"#, &[]);
        let mut def = test_agent_def("agent-1", "Agent", "claude", "agent", 1, "");
        def.memory_id = "b1".to_string();
        mstore.agent_def_insert(&mut def).unwrap();

        // b1 refs a private server, a global one, and a private one agent-1 also binds itself.
        for (id, global) in [("private", false), ("global", true), ("shared-with-agent", false), ("agent-own", false)] {
            identity.mcp_server_insert_raw(&server(id, global)).unwrap();
        }
        for id in ["private", "global", "shared-with-agent"] {
            exec(&mstore, "INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES ('b1', ?1)", &[id]);
        }
        for id in ["shared-with-agent", "agent-own"] {
            exec(&mstore, "INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('agent-1', ?1)", &[id]);
        }

        drop_bundle_mcp(&mstore, Some(&identity), &shared).unwrap();
        // Idempotent.
        drop_bundle_mcp(&mstore, Some(&identity), &shared).unwrap();

        assert_eq!(count(&mstore, "SELECT COUNT(*) FROM db_bundle_mcp_ref"), 0);
        assert_eq!(count(&mstore, "SELECT COUNT(*) FROM db_agent_mcp_ref"), 2, "the agent's own binds stay");
        assert!(identity.mcp_server_get("private").unwrap().is_none(), "a bundle-private server goes");
        for kept in ["global", "shared-with-agent", "agent-own"] {
            assert!(identity.mcp_server_get(kept).unwrap().is_some(), "{kept} stays");
        }
        assert_eq!(count(&shared, "SELECT COUNT(*) FROM db_bundles WHERE mcp_servers != '[]'"), 0, "the inline column is cleared");
    }

    #[test]
    fn without_the_identity_store_the_refs_still_go() {
        let mstore = Store::open_in_memory().unwrap();
        let shared = Store::open_in_memory().unwrap();
        exec(&mstore, "INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES ('b1', 'x')", &[]);
        drop_bundle_mcp(&mstore, None, &shared).unwrap();
        assert_eq!(count(&mstore, "SELECT COUNT(*) FROM db_bundle_mcp_ref"), 0);
    }

    /// A failed clear of the inline column fails the run, so the migration
    /// isn't recorded as applied with server configs still stored.
    #[test]
    fn a_failed_inline_clear_fails_the_migration() {
        let mstore = Store::open_in_memory().unwrap();
        let shared = Store::open_in_memory().unwrap();
        exec(&shared, "DROP TABLE db_bundles", &[]);
        let err = drop_bundle_mcp(&mstore, None, &shared).unwrap_err();
        assert!(err.contains("clear inline mcp_servers"), "{err}");
    }

    /// No fallback to the channel store's local copy: an unopenable shared
    /// store fails the run, to be retried.
    #[test]
    fn an_unopenable_shared_store_fails_the_migration() {
        let dir = tempfile::tempdir().unwrap();
        let channel = dir.path().join("objects.db");
        Store::open(&channel).unwrap();
        let ctx = MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            // A directory, which SQLite can't open as a database.
            shared_store_path: dir.path().to_path_buf(),
            channel_store_path: channel,
        };
        let err = M0036DropBundleMcp.up(&ctx).unwrap_err();
        assert!(err.0.contains("open shared store"), "{err}");
    }

    #[test]
    fn a_missing_channel_store_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: dir.path().join("shared.db"),
            channel_store_path: dir.path().join("absent.db"),
        };
        M0036DropBundleMcp.up(&ctx).unwrap();
    }
}
