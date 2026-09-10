// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Carry each channel's `db_skills` / `db_mcp_servers` rows into the
//! permanently-global identity store, which became authoritative for them in
//! Phase 2a (`OBJECT_SCHEMA_VERSION`/`IDENTITY_STORE_SCHEMA_VERSION` v6 —
//! see `SPEC_DURABLE_BINDINGS_2026_09_10.md` §7 Phase 2).
//!
//! **Must ship before any Store method redirects catalog reads to the
//! identity store.** Until this has run once for a channel, that channel's
//! skills/servers exist ONLY in its own `objects.db` — redirecting reads
//! first would make every existing skill invisible. This migration is what
//! makes the redirect (a later PR) safe to land.
//!
//! Channel-scoped, not Global: an earlier design enumerated and WROTE to
//! every sibling channel's `objects.db` in one pass (mirroring
//! `m0022_identity_store_links_backfill`'s multi-source read), but that
//! migration is explicitly documented as never mutating a source — this one
//! must, to repoint ref rows (see below), and doing that across a sibling
//! channel's live file from a process that isn't running that channel is an
//! unnecessary risk this migration doesn't need to take. Each channel
//! carries only its OWN rows, on its OWN first boot after this ships —
//! exactly the same "every channel eventually boots and converges" shape
//! `m0021`/`m0030` already use, and the only stores this ever writes to are
//! the ones every other migration in this series already writes to: the
//! current channel's own store, plus (here) the identity store.
//!
//! ## The dedup rule (§5.3)
//!
//! - **A recognized starter** (`is_global = 1`, name matches the embedded
//!   skill/MCP-server manifest) converges on the SAME id every channel would
//!   independently mint for it as of Phase 1 (`skill_seed::starter_skill_id`
//!   / `mcp_seed::starter_mcp_server_id`), recomputed from the manifest —
//!   never trusted from whatever id this particular channel's row happens to
//!   have. This is what makes convergence independent of which channel's
//!   migration happens to run first.
//! - **Any other global row** (a user-promoted global skill/server, not one
//!   of the six starters) dedups by `(name, skill_type)` / `name` against
//!   whatever the identity store already holds — first-wins. Global rows are
//!   safe to collapse across channels because a global row is owned by
//!   nobody (`SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.3).
//! - **An owner-private row** (`is_global = 0`) is never deduplicated —
//!   carried across under its own id unless that id collides with something
//!   already in the identity store, in which case it gets a fresh one.
//!   Merging two different owners' rows because they happen to share a name
//!   would create an edit channel between them that never existed.
//!
//! Whenever the identity-store id differs from this channel's own local id
//! (a legacy pre-Phase-1 starter, or a collision-forced rename), this
//! channel's OWN ref rows (`db_agent_skills_ref`/`db_bundle_skills_ref`, and
//! the MCP equivalents) are rewritten in place to the new id — see
//! `Store::skill_rewrite_ref_id`/`mcp_server_rewrite_ref_id`. Without this,
//! the ref row would name an id that resolves nowhere once catalog reads
//! move to the identity store.
//!
//! The local `db_skills`/`db_mcp_servers` rows themselves are never deleted
//! — Phase 2a already decided the channel-store copies keep existing as a
//! degraded-mode fallback, so there is nothing to clean up here, and leaving
//! a carried row in place costs nothing.
//!
//! Idempotent per row, not just per migration: every insert is `INSERT OR
//! IGNORE` keyed on the chosen id, and a row whose id already exists in the
//! identity store is treated as "already carried" and skipped outright — so
//! a re-run (this channel's next boot, or a retry after a transient I/O
//! error on one row) picks up only what is still missing rather than
//! re-deciding ids for rows it already placed.

use std::sync::Arc;

use crate::backend::skill_seed;
use crate::backend::mcp_seed;
use crate::backend::storage::store::Store;
use crate::registry;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0031CarrySkillsAndMcpServersToIdentityStore;

/// Resolve (creating the parent dir if needed) and open the identity store
/// for writing. Shared by `up()`; factored out so a resolution failure gets
/// one consistent error message.
fn open_identity_store() -> Result<Store, String> {
    let path = registry::resolve_identity_store_path()
        .ok_or_else(|| "could not resolve identity store path".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    Store::open_identity_store(&path).map_err(|e| format!("open identity store: {e}"))
}

/// Heuristic for "is this the identity store's existing row actually MY row,
/// already carried by an earlier run — not a different private skill that
/// happens to collide on id?" There is no stronger identity than content to
/// check against for a private row (ids alone can't distinguish the two
/// cases — that's the whole problem). `created_at` is included because it is
/// set once at creation and never touched by an ordinary edit, so two
/// genuinely different rows sharing it too is vanishingly unlikely.
fn skill_looks_like_the_same_row(a: &crate::backend::storage::skills::Skill, b: &crate::backend::storage::skills::Skill) -> bool {
    a.name == b.name && a.content == b.content && a.skill_type == b.skill_type && a.created_at == b.created_at
}

/// Carry this channel's skills across. Returns `(carried, rewritten)`.
/// `carried` counts only rows this call actually inserted into the
/// IDENTITY STORE for the first time — a row that converges onto an id
/// already placed there (by this channel's own earlier run, or by a
/// different channel entirely) does not increment it, even though it still
/// gets a local copy under the new id. `rewritten` counts actual ref ROWS
/// repointed, not "how many rows had an id change" — a converging row with
/// no local ref pointing at it yet contributes 0 either way.
fn carry_skills(wstore: &Store, identity_store: &Store) -> Result<(usize, usize), String> {
    let mut carried = 0usize;
    let mut rewritten = 0usize;
    for skill in wstore.skill_list_all_raw().map_err(|e| format!("list local skills: {e}"))? {
        let final_id = if skill.is_global {
            if let Some(trigger) = skill_seed::starter_skill_trigger_for_name(&skill.name) {
                skill_seed::starter_skill_id(&trigger).to_string()
            } else {
                match identity_store
                    .skill_find_global_by_name_and_type(&skill.name, &skill.skill_type)
                    .map_err(|e| format!("find global skill {}: {e}", skill.name))?
                {
                    Some(existing) => existing.id,
                    None => skill.id.clone(),
                }
            }
        } else {
            // Private: keep the own id UNLESS something else already
            // occupies it. "Something else" is content-checked, not just
            // presence-checked — an identical row already there means THIS
            // is a re-run finding its own prior work, not a collision.
            match identity_store
                .skill_get(&skill.id)
                .map_err(|e| format!("collision check for private skill {}: {e}", skill.id))?
            {
                None => skill.id.clone(),
                Some(existing) if skill_looks_like_the_same_row(&existing, &skill) => skill.id.clone(),
                Some(_) => uuid::Uuid::new_v4().to_string(),
            }
        };

        let mut to_insert = skill.clone();
        let original_id = skill.id.clone();
        to_insert.id = final_id.clone();
        let newly_inserted = identity_store
            .skill_insert_raw(&to_insert)
            .map_err(|e| format!("insert skill {}: {e}", to_insert.name))?;
        if newly_inserted > 0 {
            carried += 1;
        }

        if final_id != original_id {
            // A LOCAL row under the new id too, before touching any ref row
            // — required for two reasons, not one. `db_agent_skills_ref`'s
            // own FK (`REFERENCES db_skills(id)`) targets the LOCAL table,
            // so rewriting a ref to an id that exists only in the identity
            // store fails outright. And even where that FK isn't enforced,
            // ordinary (pre-redirect) code still resolves a ref's skill_id
            // against the LOCAL catalog — without a local row under the new
            // id, every existing binding would read as broken from the
            // moment this migration runs until the separate PR that
            // redirects reads to the identity store ships. `OR IGNORE`
            // keeps this idempotent across re-runs of the same rename.
            wstore
                .skill_insert_raw(&to_insert)
                .map_err(|e| format!("insert local copy under new id for skill {}: {e}", to_insert.name))?;
            rewritten += wstore
                .skill_rewrite_ref_id(&original_id, &final_id)
                .map_err(|e| format!("rewrite refs for skill {}: {e}", to_insert.name))?;
        }
    }
    Ok((carried, rewritten))
}

/// Mirrors `skill_looks_like_the_same_row` for MCP servers. `config` (not
/// `content` — servers have no such field) plays the same "reliable enough
/// to distinguish, not so strict a trivial edit breaks it" role.
fn mcp_server_looks_like_the_same_row(a: &crate::backend::storage::mcp_servers::McpServer, b: &crate::backend::storage::mcp_servers::McpServer) -> bool {
    a.name == b.name && a.config == b.config && a.transport == b.transport && a.created_at == b.created_at
}

/// Mirrors `carry_skills` exactly, for MCP servers — see that function's
/// comments for the reasoning behind every decision here.
fn carry_mcp_servers(wstore: &Store, identity_store: &Store) -> Result<(usize, usize), String> {
    let mut carried = 0usize;
    let mut rewritten = 0usize;
    for server in wstore.mcp_server_list_all_raw().map_err(|e| format!("list local mcp servers: {e}"))? {
        let final_id = if server.is_global {
            if mcp_seed::is_starter_mcp_server_name(&server.name) {
                mcp_seed::starter_mcp_server_id(&server.name).to_string()
            } else {
                match identity_store
                    .mcp_server_find_global_by_name(&server.name)
                    .map_err(|e| format!("find global mcp server {}: {e}", server.name))?
                {
                    Some(existing) => existing.id,
                    None => server.id.clone(),
                }
            }
        } else {
            match identity_store
                .mcp_server_get(&server.id)
                .map_err(|e| format!("collision check for private mcp server {}: {e}", server.id))?
            {
                None => server.id.clone(),
                Some(existing) if mcp_server_looks_like_the_same_row(&existing, &server) => server.id.clone(),
                Some(_) => uuid::Uuid::new_v4().to_string(),
            }
        };

        let mut to_insert = server.clone();
        let original_id = server.id.clone();
        to_insert.id = final_id.clone();
        let newly_inserted = identity_store
            .mcp_server_insert_raw(&to_insert)
            .map_err(|e| format!("insert mcp server {}: {e}", to_insert.name))?;
        if newly_inserted > 0 {
            carried += 1;
        }

        if final_id != original_id {
            // See carry_skills's matching comment — same FK and same
            // deploy-window reasoning, mirrored for MCP servers.
            wstore
                .mcp_server_insert_raw(&to_insert)
                .map_err(|e| format!("insert local copy under new id for mcp server {}: {e}", to_insert.name))?;
            rewritten += wstore
                .mcp_server_rewrite_ref_id(&original_id, &final_id)
                .map_err(|e| format!("rewrite refs for mcp server {}: {e}", to_insert.name))?;
        }
    }
    Ok((carried, rewritten))
}

impl Migration for M0031CarrySkillsAndMcpServersToIdentityStore {
    fn id(&self) -> &'static str {
        "0031_carry_skills_and_mcp_servers_to_identity_store"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Carry this channel's skills/MCP servers into the identity store, which is now authoritative for them"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: open wstore: {e}")))?,
        );
        let identity_store = open_identity_store()
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: {e}")))?;

        let (skills_carried, skills_rewritten) = carry_skills(&wstore, &identity_store)
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: skills: {e}")))?;
        let (servers_carried, servers_rewritten) = carry_mcp_servers(&wstore, &identity_store)
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: mcp servers: {e}")))?;

        tracing::info!(
            skills_carried,
            skills_refs_rewritten = skills_rewritten,
            mcp_servers_carried = servers_carried,
            mcp_servers_refs_rewritten = servers_rewritten,
            "carry_skills_and_mcp_servers_to_identity_store: complete"
        );
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let local = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0))
                    .unwrap_or(0);
                let servers: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_mcp_servers", [], |r| r.get(0))
                    .unwrap_or(0);
                format!("{skills} local skill row(s), {servers} local mcp server row(s)")
            }
            Ok(None) => "no channel store".to_string(),
            Err(e) => return VerifyOutcome::Error(e),
        };

        // Best-effort: the identity store is host-global and this migration
        // is per-channel, so a resolution failure here says nothing about
        // whether THIS channel's own carry succeeded — report what local
        // state shows rather than failing verification over it.
        let Some(identity_path) = registry::resolve_identity_store_path() else {
            return VerifyOutcome::Ok(format!("{local}; identity store path unresolved"));
        };
        match super::runner::open_readonly(&identity_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0))
                    .unwrap_or(0);
                let servers: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_mcp_servers", [], |r| r.get(0))
                    .unwrap_or(0);
                VerifyOutcome::Ok(format!(
                    "{local}; identity store has {skills} skill row(s), {servers} mcp server row(s)"
                ))
            }
            Ok(None) => VerifyOutcome::Ok(format!("{local}; identity store not yet created")),
            Err(e) => VerifyOutcome::Ok(format!("{local}; identity store unreadable: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::mcp_servers::McpServer;
    use crate::backend::storage::skills::Skill;

    /// Only exercised for the missing-channel-store no-op path below, which
    /// returns before `open_identity_store()` is ever called — so this
    /// deliberately does NOT set up identity-store resolution (there is no
    /// real per-call override for it; `resolve_identity_store_path` only
    /// respects the global `AGENTMUX_HOME_OVERRIDE`, which every other test
    /// in this file avoids touching to stay independent of process-wide
    /// env-var state).
    fn ctx_for(channel_path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-shared-store.db"),
            channel_store_path: channel_path.to_path_buf(),
        }
    }

    fn skill(id: &str, name: &str, trigger: &str, is_global: bool) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: trigger.to_string(),
            skill_type: "prompt".to_string(),
            description: "d".to_string(),
            content: "c".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn mcp(id: &str, name: &str, is_global: bool) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn a_recognized_starter_converges_on_the_deterministic_id_regardless_of_its_legacy_local_id() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        // A legacy pre-Phase-1 row: real starter trigger, but a random id
        // that does NOT match starter_skill_id("tdd").
        wstore.skill_insert_raw(&skill("legacy-random-id", "Test-Driven Development", "tdd", true)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let (carried, rewritten) = carry_skills(&wstore, &identity_store).unwrap();
        assert_eq!(carried, 1);
        assert_eq!(rewritten, 0, "no ref rows existed locally, so nothing to rewrite");

        let expected_id = skill_seed::starter_skill_id("tdd").to_string();
        assert!(
            identity_store.skill_get(&expected_id).unwrap().is_some(),
            "must land under the canonical deterministic id, not the legacy random one"
        );
        assert!(identity_store.skill_get("legacy-random-id").unwrap().is_none());
    }

    #[test]
    fn ref_rows_are_rewritten_when_the_id_changes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let wstore = Store::open(&db_path).unwrap();
        wstore.skill_insert_raw(&skill("legacy-random-id", "Test-Driven Development", "tdd", true)).unwrap();
        // A real ref row naming the legacy id — the thing that must not be
        // left dangling once the migration mints a different id. `Store.conn`
        // isn't reachable from this module (`pub(super)`), so this goes
        // through a second raw connection to the same file — the same
        // pattern m0025/m0026/m0027's own tests already use to seed
        // db_agents rows directly.
        rusqlite::Connection::open(&db_path).unwrap().execute(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')",
            [],
        ).unwrap();
        wstore.skill_bind("agent-1", "legacy-random-id").unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        let (_, rewritten) = carry_skills(&wstore, &identity_store).unwrap();
        assert_eq!(rewritten, 1);

        let expected_id = skill_seed::starter_skill_id("tdd").to_string();
        assert!(
            wstore.skill_is_bound_to("agent-1", &expected_id).unwrap(),
            "the agent's own ref row must now point at the canonical id"
        );
        assert!(!wstore.skill_is_bound_to("agent-1", "legacy-random-id").unwrap());
    }

    #[test]
    fn a_non_starter_global_skill_dedups_by_name_and_type_first_wins() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        // Channel A carries first: its id survives.
        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        wstore_a.skill_insert_raw(&skill("id-from-a", "My Custom Global", "", true)).unwrap();
        carry_skills(&wstore_a, &identity_store).unwrap();

        // Channel B carries the SAME name+type, under a different id — must
        // converge onto A's id, not create a second row.
        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        wstore_b.skill_insert_raw(&skill("id-from-b", "My Custom Global", "", true)).unwrap();
        let (carried, rewritten) = carry_skills(&wstore_b, &identity_store).unwrap();
        assert_eq!(carried, 0, "id-from-a already occupies the identity store under its own id — B adds nothing new there");
        // `rewritten` counts actual ref ROWS repointed, not "an id changed" —
        // this fixture never binds "id-from-b" to any agent/bundle, so there
        // is nothing to rewrite even though B's local copy does converge
        // onto A's id (see `ref_rows_are_rewritten_when_the_id_changes` for
        // the case where a real ref row exists).
        assert_eq!(rewritten, 0);
        assert!(
            wstore_b.skill_get("id-from-a").unwrap().is_some(),
            "B's own local store gets a copy under A's id too, so pre-redirect code keeps resolving it"
        );

        assert!(identity_store.skill_get("id-from-a").unwrap().is_some());
        assert!(identity_store.skill_get("id-from-b").unwrap().is_none(), "must not create a second row for the same name+type");
    }

    #[test]
    fn owner_private_skills_are_never_deduplicated_by_name() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        wstore_a.skill_insert_raw(&skill("private-a", "Deploy", "", false)).unwrap();
        carry_skills(&wstore_a, &identity_store).unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        wstore_b.skill_insert_raw(&skill("private-b", "Deploy", "", false)).unwrap();
        carry_skills(&wstore_b, &identity_store).unwrap();

        // Both rows survive as distinct resources — merging two different
        // owners' private skills because they share a name would create an
        // edit channel between them that never existed.
        assert!(identity_store.skill_get("private-a").unwrap().is_some());
        assert!(identity_store.skill_get("private-b").unwrap().is_some());
    }

    #[test]
    fn a_private_skill_id_collision_gets_a_fresh_id_and_its_refs_are_rewritten() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        // Pre-seed the identity store with a row under the SAME id channel
        // B's private skill happens to have — the collision case.
        identity_store.skill_insert_raw(&skill("collided-id", "Someone Else's Skill", "", false)).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let wstore = Store::open(&db_path).unwrap();
        wstore.skill_insert_raw(&skill("collided-id", "My Private Skill", "", false)).unwrap();
        rusqlite::Connection::open(&db_path).unwrap().execute(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')",
            [],
        ).unwrap();
        wstore.skill_bind("agent-1", "collided-id").unwrap();

        let (carried, rewritten) = carry_skills(&wstore, &identity_store).unwrap();
        assert_eq!(carried, 1);
        assert_eq!(rewritten, 1);

        // The pre-existing row under "collided-id" must be untouched.
        let existing = identity_store.skill_get("collided-id").unwrap().unwrap();
        assert_eq!(existing.name, "Someone Else's Skill");
        // The agent's ref must now point at whatever fresh id was minted,
        // not at "collided-id".
        assert!(!wstore.skill_is_bound_to("agent-1", "collided-id").unwrap());
    }

    #[test]
    fn re_running_after_a_partial_carry_only_carries_what_is_still_missing() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.skill_insert_raw(&skill("s1", "Skill One", "", false)).unwrap();
        wstore.skill_insert_raw(&skill("s2", "Skill Two", "", false)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let (first_pass, _) = carry_skills(&wstore, &identity_store).unwrap();
        assert_eq!(first_pass, 2);

        let (second_pass, _) = carry_skills(&wstore, &identity_store).unwrap();
        assert_eq!(second_pass, 0, "a full re-run must find nothing left to carry");
    }

    #[test]
    fn a_recognized_starter_mcp_server_converges_on_the_deterministic_id() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.mcp_server_insert_raw(&mcp("legacy-random-id", "git", true)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        carry_mcp_servers(&wstore, &identity_store).unwrap();

        let expected_id = mcp_seed::starter_mcp_server_id("git").to_string();
        assert!(identity_store.mcp_server_get(&expected_id).unwrap().is_some());
        assert!(identity_store.mcp_server_get("legacy-random-id").unwrap().is_none());
    }

    #[test]
    fn a_non_existent_channel_store_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir.path().join("does-not-exist.db"));
        let result = M0031CarrySkillsAndMcpServersToIdentityStore.up(&ctx);
        assert!(result.is_ok());
    }
}
