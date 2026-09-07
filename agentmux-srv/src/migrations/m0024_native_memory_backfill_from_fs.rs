// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Finish the job `m0023` was meant to do, and repair the label it left
//! behind — `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
//! §9 (rollout backfill) and §4.5 (drift detection).
//!
//! ## What went wrong
//!
//! §9 asks for "a one-time backfill [of] a single `source: 'agent_inferred'`
//! version per existing `(agent_id, filename)` row ... so history views
//! aren't empty for pre-existing memory on upgrade". `m0023` implements
//! that literally, iterating `db_agent_native_memory` — the **mirror**
//! table. The spec's own wording ("per existing ... row") is what led
//! there, and it is wrong in practice for one reason: that mirror is
//! populated lazily, only once something reads or exports an agent's
//! memory. On a typical install it is EMPTY, so `m0023` iterates nothing
//! and backfills nothing, in 0 ms.
//!
//! The drift sweep (`backend::native_memory_drift`) does not use the
//! mirror — it reads the real filesystem, which is authoritative here
//! (`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`). Starting moments
//! after migrations finish, it finds those same files, sees no recorded
//! version, and records each as `external_fs_write` — rendered in Armory
//! as **"Detected outside AgentMux — provenance unknown"**.
//!
//! Net effect on every install that upgraded with an unpopulated mirror:
//! ordinary pre-existing memory files are permanently tagged with a
//! security-flavored warning, and the benign backfill the spec asked for
//! never happened. Observed on a real install: `m0023` applied at
//! `…285933` (0 ms, 0 rows, mirror empty), 21 `external_fs_write` rows
//! written across 6 agents and 16 files 61 ms later at `…285994`.
//!
//! This is NOT a race — `bootstrap` runs migrations to completion before
//! the drift tasks spawn. Ordering was never the problem; reading the
//! wrong source was. That is why the fix below is a migration and not a
//! change to startup sequencing.
//!
//! ## What this does
//!
//! 1. **Backfill from the filesystem**, the same source the detector
//!    trusts, via the same `list_all_memory_targets` enumeration — so the
//!    two agree by construction instead of by coincidence. Idempotent and
//!    ordering-safe for exactly the reason `m0023` documents: only a pair
//!    with NO version yet is touched, so a pair written normally since
//!    upgrade never gets an out-of-chronology row appended.
//! 2. **Repair the mislabeled rows** already written by the sweep, under a
//!    deliberately narrow predicate — see
//!    `agent_native_memory_version_relabel_rollout_first_sight`, which
//!    documents each clause. It cannot touch a genuine external write.
//!
//! ## Why this is Channel-scoped, and which store enumerates
//!
//! The two stores are not interchangeable, and picking the wrong one here
//! would have reproduced the exact bug this migration exists to fix
//! (caught in review before merge, not by me):
//!
//! - **Versions** (`db_agent_native_memory_versions`) live in the SHARED
//!   store, so every read/insert/relabel below uses `shared_store_path`.
//! - **Enumeration** does not. `list_all_memory_targets` calls
//!   `agent_def_list()`, which prepares against `db_agents`, and
//!   `memory_dir_for_agent_by_id`, which reads `db_agent_content` — both
//!   per-channel tables created by `run_object_schema`, absent from
//!   `Store::open_shared` by design (see its doc comment). Handing it the
//!   shared store makes `agent_def_list()` fail with a SQL error that
//!   `list_all_memory_targets` swallows (`if let Ok(agents)`), so that
//!   whole enumeration path silently yields nothing and only currently
//!   *active* registry agents get backfilled — every persisted-but-idle
//!   agent missed, which is the same "read the wrong source, get an empty
//!   list, look successful" failure as `m0023`.
//!
//! So the migration is `Channel`-scoped: it runs once per channel, with
//! that channel's own store doing the enumeration — matching what the
//! drift sweep itself gets in production (`state.wstore` is the channel
//! store) — while still reading and writing versions in the shared store.
//! The repair half is idempotent, so running once per channel is safe.
//!
//! Order matters: repair runs FIRST. Relabeling turns a row into the
//! `agent_inferred` version the pair should have had, which then makes
//! step 2's "has no version yet" test correctly skip that pair. Backfilling
//! first would leave the pair versioned but still mislabeled, and the
//! repair's `parent_version_id IS NULL` clause would no longer match.

use std::sync::Arc;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0024NativeMemoryBackfillFromFs;

impl Migration for M0024NativeMemoryBackfillFromFs {
    fn id(&self) -> &'static str { "0024_native_memory_backfill_from_fs" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Backfill native memory versions from disk; repair mislabeled rollout rows"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let store = Arc::new(
            Store::open_shared(&ctx.shared_store_path).map_err(|e| {
                MigrationError(format!("native_memory_backfill_from_fs: open shared store: {e}"))
            })?,
        );

        // ── 1. Repair first (see the module doc for why the order matters).
        let relabeled = store
            .agent_native_memory_version_relabel_rollout_first_sight()
            .map_err(|e| {
                MigrationError(format!("native_memory_backfill_from_fs: relabel: {e}"))
            })?;

        // ── 2. Backfill anything still unversioned, reading the filesystem.
        // Enumerate with the CHANNEL store — see the module doc. Absent on
        // a channel whose objects.db hasn't been created yet, in which case
        // there are no agents here to backfill and the repair above was the
        // whole job.
        if !ctx.channel_store_path.exists() {
            tracing::info!(
                relabeled,
                "native_memory_backfill_from_fs: no channel store yet; repair-only pass"
            );
            return Ok(());
        }
        let channel_store = Store::open(&ctx.channel_store_path).map_err(|e| {
            MigrationError(format!("native_memory_backfill_from_fs: open channel store: {e}"))
        })?;
        let targets =
            crate::server::native_memory_handlers::list_all_memory_targets(&channel_store);
        let mut backfilled = 0usize;
        let mut skipped_already_versioned = 0usize;
        let mut unreadable = 0usize;

        for (agent_id, dir) in targets {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                // A configured agent whose memory dir doesn't exist yet is
                // ordinary, not an error. Same tolerance the sweep has.
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let filename = entry.file_name().to_string_lossy().into_owned();
                if !filename.ends_with(".md") {
                    continue;
                }
                if !entry.path().is_file() {
                    continue;
                }
                let existing = store
                    .agent_native_memory_version_latest(&agent_id, &filename)
                    .map_err(|e| {
                        MigrationError(format!(
                            "native_memory_backfill_from_fs: check existing for {agent_id}/{filename}: {e}"
                        ))
                    })?;
                if existing.is_some() {
                    skipped_already_versioned += 1;
                    continue;
                }
                // Oversized/unreadable files are skipped, never truncated:
                // recording a partial body as a *version* would misrepresent
                // content the file never held as authoritative (the same
                // reasoning `read_memory_file_lossy` documents). One bad file
                // must not abort the backfill for every other agent.
                let content = match crate::backend::native_memory_drift::read_memory_file_lossy(
                    &entry.path(),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            agent_id, filename, error = %e,
                            "native_memory_backfill_from_fs: skipping unreadable file"
                        );
                        unreadable += 1;
                        continue;
                    }
                };
                store
                    .agent_native_memory_version_insert(
                        &agent_id, &filename, &content, "agent_inferred", "{}", "",
                    )
                    .map_err(|e| {
                        MigrationError(format!(
                            "native_memory_backfill_from_fs: insert for {agent_id}/{filename}: {e}"
                        ))
                    })?;
                backfilled += 1;
            }
        }

        tracing::info!(
            relabeled, backfilled, skipped_already_versioned, unreadable,
            "native_memory_backfill_from_fs: complete"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(shared_store_path: std::path::PathBuf) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path,
            channel_store_path: std::env::temp_dir().join("unused-objects.db"),
        }
    }

    /// Record that `m0023` ran just now, which is what puts a version
    /// inserted by a test inside the repair's rollout window. Writes
    /// through its own connection to the same file rather than the
    /// `Store`, whose `conn` is private to `backend::storage`.
    fn record_m0023_now(db_path: &std::path::Path) {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let applied_at: String = conn
            .query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%S+00:00', ?1, 'unixepoch')",
                rusqlite::params![secs],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO db_migrations (id, applied_at, duration_ms, scope)
             VALUES ('0023_native_memory_versions_backfill', ?1, 0, 'global')",
            rusqlite::params![applied_at],
        )
        .unwrap();
    }

    /// The real-world regression, end to end at the migration level: an
    /// install where m0023 backfilled nothing and the sweep then claimed
    /// the pre-existing files as "detected outside AgentMux".
    #[test]
    fn repairs_the_mislabeled_rollout_rows() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        record_m0023_now(tmp.path());
        store
            .agent_native_memory_version_insert(
                "agent-1", "MEMORY.md", "pre-existing", "external_fs_write",
                r#"{"detected_via":"reconciliation_sweep"}"#, "",
            )
            .unwrap();

        M0024NativeMemoryBackfillFromFs.up(&ctx_for(tmp.path().to_path_buf())).unwrap();

        let versions = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(versions.len(), 1, "repair must relabel in place, never append a row");
        assert_eq!(versions[0].source, "agent_inferred");
    }

    /// Running it twice must not double-apply or append anything — an
    /// operator re-running migrations is routine.
    #[test]
    fn is_idempotent_on_a_second_run() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        record_m0023_now(tmp.path());
        store
            .agent_native_memory_version_insert(
                "agent-1", "MEMORY.md", "pre-existing", "external_fs_write",
                r#"{"detected_via":"reconciliation_sweep"}"#, "",
            )
            .unwrap();

        let ctx = ctx_for(tmp.path().to_path_buf());
        M0024NativeMemoryBackfillFromFs.up(&ctx).unwrap();
        M0024NativeMemoryBackfillFromFs.up(&ctx).unwrap();

        let versions = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].source, "agent_inferred");
    }

    /// The regression the review caught before merge: enumeration must use
    /// the CHANNEL store. With the shared store, `agent_def_list()` fails on
    /// the missing `db_agents` table, `list_all_memory_targets` swallows the
    /// error, and this agent's on-disk memory is silently never backfilled —
    /// the same "wrong source, empty list, looks fine" failure as m0023.
    #[test]
    fn backfills_an_on_disk_file_for_an_agent_only_the_channel_store_knows() {
        let tmp = tempfile::tempdir().unwrap();
        let shared_path = tmp.path().join("shared.db");
        let channel_path = tmp.path().join("objects.db");
        let shared = Store::open_shared(&shared_path).unwrap();
        let channel = Store::open(&channel_path).unwrap();

        // An agent whose memory dir resolves via working_directory +
        // CLAUDE_CONFIG_DIR — both per-channel reads.
        let mut def = crate::backend::storage::agents::test_agent_def(
            "agent-1", "Agent One", "claude", "agent", 1000, "",
        );
        def.working_directory = "/wd".to_string();
        channel.agent_def_insert(&mut def).unwrap();
        channel
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: def.id.clone(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", tmp.path().display()),
                updated_at: 1000,
            })
            .unwrap();

        // memory_dir_for_cwd maps "/wd" -> "-wd" (non-alphanumerics to '-').
        let mem_dir = tmp.path().join("projects").join("-wd").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), "content that predates version tracking").unwrap();

        M0024NativeMemoryBackfillFromFs
            .up(&MigrationContext {
                home: tmp.path().to_path_buf(),
                data_dir: tmp.path().to_path_buf(),
                shared_store_path: shared_path.clone(),
                channel_store_path: channel_path.clone(),
            })
            .unwrap();

        let latest = shared
            .agent_native_memory_version_latest(&def.id, "MEMORY.md")
            .unwrap()
            .expect("the on-disk file must be backfilled, not silently skipped");
        assert_eq!(latest.source, "agent_inferred");
        assert_eq!(latest.content, "content that predates version tracking");
    }

    /// A store with no shared file at all must not error — the migration
    /// runner calls every migration on every install.
    #[test]
    fn is_a_no_op_when_the_shared_store_does_not_exist() {
        let missing = std::env::temp_dir().join("agentmux-m0024-does-not-exist.db");
        let _ = std::fs::remove_file(&missing);
        M0024NativeMemoryBackfillFromFs.up(&ctx_for(missing)).unwrap();
    }
}
