// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md Phase 5 — the framework-level
//! regression tests the spec lists, run against the REAL registry (not fakes)
//! in a throwaway `~/.agentmux`:
//!
//! 1. the incident itself — a stale consolidate marker beside an empty target
//!    table — as a permanent regression test at the `m0000` + runner level
//!    (Phase 0a's acceptance, promoted);
//! 2. a crash between a migration's effect and its `db_migrations` mark, and
//!    the next boot resuming without duplicating anything — the claim
//!    `agents_consolidate.rs` makes in a comment, turned into a test;
//! 3. an upgrade that skips many versions — only the first six recorded —
//!    applying every later migration once, in registry order;
//! 4. `m0000_bootstrap`'s stamping on the pre-framework data-dir shapes.
//!
//! (The concurrent-boot item shipped with Phase 3 in `runner::lock_tests`.)
//!
//! Every test here sets `AGENTMUX_HOME_OVERRIDE` so the env-resolved paths
//! the migrations use (`resolve_shared_registry_dir` and friends) land inside
//! the temp home, and so takes the crate-wide `ISOLATED_AUTH_ENV_LOCK` — the
//! same discipline `registry::paths`' and `runner`'s own env tests follow.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use rusqlite::{params, Connection};

use crate::backend::storage::store::Store;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

use super::m0000_bootstrap::M0000Bootstrap;
use super::runner::{apply_pending, read_applied_ids, verify_applied, AppliedIds, ApplyProgress};
use super::{Migration, MigrationContext, MigrationScope, VerifyOutcome, REGISTRY};

const CONSOLIDATE_ID: &str = "0007_agents_consolidate";
const CONSOLIDATE_FLAG: &str = "migration_agents_consolidate_v1.flag";

/// A throwaway `~/.agentmux`. Holds the env lock for its lifetime, so tests
/// in this module (and every other env-mutating test in the crate) serialise.
struct TempHome {
    dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

fn clear_env() {
    for var in [
        "AGENTMUX_HOME_OVERRIDE",
        "AGENTMUX_DATA_DIR",
        "AGENTMUX_SHARED_DIR",
        "AGENTMUX_ISOLATED_AUTH",
        "AGENTMUX_INSTANCE_DIR",
        "AGENTMUX_CHANNEL",
    ] {
        std::env::remove_var(var);
    }
}

impl TempHome {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", dir.path());
        std::fs::create_dir_all(dir.path().join("shared")).unwrap();
        std::fs::create_dir_all(dir.path().join("data").join("db")).unwrap();
        Self { dir, _guard: guard }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }
    fn shared_store(&self) -> PathBuf {
        self.home().join("shared").join("store.db")
    }
    fn data_dir(&self) -> PathBuf {
        self.home().join("data")
    }
    fn channel_store(&self) -> PathBuf {
        self.data_dir().join("db").join("objects.db")
    }
    fn ctx(&self) -> MigrationContext {
        MigrationContext {
            home: self.home().to_path_buf(),
            data_dir: self.data_dir(),
            shared_store_path: self.shared_store(),
            channel_store_path: self.channel_store(),
        }
    }

    /// Marks this home as a real prior-usage install (`<home>/agents/` is the
    /// proxy `m0000`/`m0001` use), which also keeps `m0001` from looking at
    /// the developer's real `~/.waveterm`.
    fn mark_as_existing_install(&self) {
        std::fs::create_dir_all(self.home().join("agents")).unwrap();
    }

    fn apply_all(&self, progress: Option<&ApplyProgress<'_>>) -> super::runner::ApplyOutcome {
        apply_pending(REGISTRY, self.home(), &self.shared_store(), &self.data_dir(), progress)
            .expect("apply_pending against the real registry")
    }

    fn applied(&self, scope: MigrationScope) -> Vec<String> {
        let path = match scope {
            MigrationScope::Global => self.shared_store(),
            MigrationScope::Channel => self.channel_store(),
        };
        let mut ids: Vec<String> = read_applied_ids(&path).expect("readable tracking state").into_iter().collect();
        ids.sort();
        ids
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        clear_env();
    }
}

impl TempHome {
    /// The `CLAUDE_CONFIG_DIR` every fixture agent runs under — inside the
    /// temp home, so `m0024_native_memory_backfill_from_fs` (which reads
    /// each agent's memory directory off disk during the full-registry
    /// tests) resolves to `<temp>/shared/providers/claude/...` and never to
    /// the developer's real `~/.agentmux/shared/providers/claude`. That
    /// resolver (`memory_dir_for_cwd`) falls back to the REAL home when an
    /// agent has no `CLAUDE_CONFIG_DIR` and to `~/.agentmux/agents/<name>`
    /// when its working directory is blank, so both are set explicitly
    /// (codex P2 on #3066).
    fn claude_config_dir(&self) -> PathBuf {
        self.home().join("shared").join("providers").join("claude")
    }

    /// A fixture agent's working directory — also inside the temp home.
    fn agent_working_dir(&self, id: &str) -> PathBuf {
        self.home().join("agents").join(id)
    }

    /// Where Claude Code keeps the memory files for `agent_working_dir(id)`
    /// under `claude_config_dir()` — the same sanitisation
    /// `memory_dir_for_cwd` applies (every non-alphanumeric byte → `-`).
    fn agent_memory_dir(&self, id: &str) -> PathBuf {
        let folder: String = self
            .agent_working_dir(id)
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.claude_config_dir().join("projects").join(folder).join("memory")
    }

    /// A legacy `db_agent_definitions` row — the pre-consolidation shape
    /// `0007` exists to fold into `db_agents` — plus the `env` content row
    /// that pins its `CLAUDE_CONFIG_DIR` inside this temp home. Same
    /// definition columns the consolidate module's own tests insert.
    fn insert_legacy_definition(&self, id: &str, name: &str) {
        // `Store::open` lays down the schema (including the legacy tables it
        // still adopts); the rows go in through SQL because no live API
        // writes that table any more.
        let channel_store = self.channel_store();
        drop(Store::open(&channel_store).expect("create channel store"));
        let conn = Connection::open(&channel_store).unwrap();
        // `db_agent_definitions` is gone from the base schema entirely as of
        // v32 (`OBJECT_SCHEMA_VERSION`'s v32 doc comment) — this fixture is
        // deliberately pre-0007/pre-consolidation, so it stands the legacy
        // table up by hand rather than relying on `Store::open` for it.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS db_agent_definitions (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL DEFAULT '',
                icon TEXT NOT NULL DEFAULT '', provider TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
                working_directory TEXT NOT NULL DEFAULT '', shell TEXT NOT NULL DEFAULT '',
                provider_flags TEXT NOT NULL DEFAULT '', auto_start INTEGER NOT NULL DEFAULT 0,
                restart_on_crash INTEGER NOT NULL DEFAULT 0, idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0, agent_type TEXT NOT NULL DEFAULT '',
                environment TEXT NOT NULL DEFAULT '', agent_bus_id TEXT NOT NULL DEFAULT '',
                is_seeded INTEGER NOT NULL DEFAULT 0, accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL DEFAULT 0, user_hidden INTEGER NOT NULL DEFAULT 0
             );",
        )
        .unwrap();
        let working_directory = self.agent_working_dir(id).to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO db_agent_definitions
                (id, slug, name, icon, provider, description, working_directory, shell,
                 provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                 created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                 parent_id, branch_label, updated_at)
             VALUES (?1, ?1, ?2, '✦', 'claude', 'desc', ?3, 'bash',
                     '', 0, 0, 0,
                     1000, 'standalone', '', '', 1, '',
                     '', '', 1000)",
            params![id, name, working_directory],
        )
        .unwrap();
        // `db_agent_content`'s agent_id FK targets db_agents as of Phase 3c —
        // but this fixture is deliberately pre-0007 in every respect,
        // `db_agents` is meant to start genuinely empty for several of these
        // tests, and 0007 (not this helper) is what's supposed to create the
        // mirror. FK checks off for just this insert, matching the same
        // pre-existing-inconsistency workaround
        // `m0028_agent_child_tables_repoint_fk`'s own tests use.
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        conn.execute(
            "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at) VALUES (?1, 'env', ?2, 1000)",
            params![id, format!("CLAUDE_CONFIG_DIR={}\n", self.claude_config_dir().display())],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    }
}

fn count(channel_store: &Path, table: &str) -> i64 {
    let conn = Connection::open(channel_store).unwrap();
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

fn registry_ids() -> Vec<&'static str> {
    REGISTRY.iter().map(|m| m.id()).collect()
}

fn scope_of(id: &str) -> MigrationScope {
    REGISTRY.iter().find(|m| m.id() == id).map(|m| m.scope()).expect("registered id")
}

// ── 4. m0000_bootstrap stamping ────────────────────────────────────────────────

#[test]
fn bootstrap_on_a_fresh_install_stamps_nothing_and_creates_no_channel_store() {
    let home = TempHome::new();
    M0000Bootstrap.up(&home.ctx()).expect("bootstrap runs on a fresh install");
    assert!(home.applied(MigrationScope::Global).is_empty(), "nothing pre-framework to stamp");
    assert!(!home.channel_store().exists(), "bootstrap must not conjure a channel store");
}

#[test]
fn bootstrap_stamps_every_pre_framework_marker_it_recognises() {
    let home = TempHome::new();
    home.mark_as_existing_install();
    // Channel-scoped markers: the zones flag, and a consolidate flag backed
    // by a genuinely populated db_agents (Phase 0a's content check passes).
    std::fs::write(home.data_dir().join("migration_agent_zones_v1.flag"), b"1").unwrap();
    home.insert_legacy_definition("tpl-1", "Coder");
    Store::open(&home.channel_store()).unwrap().run_agents_consolidate(None).unwrap();
    assert_eq!(count(&home.channel_store(), "db_agents"), 1);
    std::fs::write(home.data_dir().join(CONSOLIDATE_FLAG), b"phase3a").unwrap();
    // Global markers, under the (overridden) shared root.
    let shared_agents = home.home().join("shared").join("agents");
    for (dir, marker) in [
        ("registry", ".migrated_from_sqlite"),
        ("registry", ".backfilled_source_bases"),
        ("definitions", ".migrated_definitions"),
        ("transcripts", ".transcripts_backfilled"),
    ] {
        std::fs::create_dir_all(shared_agents.join(dir)).unwrap();
        std::fs::write(shared_agents.join(dir).join(marker), b"1").unwrap();
    }

    M0000Bootstrap.up(&home.ctx()).expect("bootstrap");

    assert_eq!(
        home.applied(MigrationScope::Global),
        vec![
            "0001_legacy_data_dir",
            "0004_registry_from_sqlite",
            "0005_registry_source_bases",
            "0006_definitions_global",
            "0009_transcript_backfill",
            "0010_session_ids",
        ],
        "0011 stays unstamped with no identity accounts; 0008 is never stamped by design"
    );
    assert_eq!(
        home.applied(MigrationScope::Channel),
        vec!["0002_block_zones_v1", "0003_template_sessions_v1", CONSOLIDATE_ID]
    );
}

#[test]
fn bootstrap_is_idempotent_on_a_second_run() {
    let home = TempHome::new();
    home.mark_as_existing_install();
    std::fs::write(home.data_dir().join("migration_agent_zones_v1.flag"), b"1").unwrap();
    drop(Store::open(&home.channel_store()).unwrap());
    M0000Bootstrap.up(&home.ctx()).unwrap();
    let first = (home.applied(MigrationScope::Global), home.applied(MigrationScope::Channel));
    M0000Bootstrap.up(&home.ctx()).unwrap();
    let second = (home.applied(MigrationScope::Global), home.applied(MigrationScope::Channel));
    assert_eq!(first, second);
    assert_eq!(first.1, vec!["0002_block_zones_v1", "0003_template_sessions_v1"]);
}

// ── 1. The incident: stale marker, empty target ───────────────────────────────

#[test]
fn a_stale_consolidate_marker_beside_an_empty_target_is_not_stamped_and_the_next_boot_backfills() {
    // The shape behind the whole hardening spec (§1, F1/F2): a rebuilt or
    // restored objects.db sitting next to a flag file that says "done". Before
    // Phase 0a, m0000 would stamp 0007 applied off the flag alone and the
    // backfill would never run again — db_agents empty forever.
    let home = TempHome::new();
    home.mark_as_existing_install();
    home.insert_legacy_definition("tpl-1", "Coder");
    home.insert_legacy_definition("tpl-2", "Reviewer");
    std::fs::write(home.data_dir().join(CONSOLIDATE_FLAG), b"phase3a").unwrap();
    assert_eq!(count(&home.channel_store(), "db_agents"), 0, "the target must start empty");

    // Bootstrap alone: the flag is present, the content check fails, so 0007
    // is left pending.
    M0000Bootstrap.up(&home.ctx()).unwrap();
    assert!(
        !home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()),
        "a stale marker must not be stamped as applied"
    );

    // The real boot: the runner sees 0007 pending and does the work.
    let outcome = home.apply_all(None);
    assert!(outcome.applied > 0);
    assert_eq!(count(&home.channel_store(), "db_agents"), 2, "the backfill actually ran");
    assert!(home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()));

    // And the doctor agrees: recorded applied AND the effect is there.
    let applied = AppliedIds {
        global: read_applied_ids(&home.shared_store()),
        channel: read_applied_ids(&home.channel_store()),
    };
    let record = verify_applied(REGISTRY, &home.ctx(), &applied)
        .into_iter()
        .find(|r| r.id == CONSOLIDATE_ID)
        .expect("0007 is verified once applied");
    match record.outcome {
        VerifyOutcome::Ok(detail) => assert!(detail.contains("db_agents=2"), "{detail}"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn a_stale_zones_flag_beside_unmigrated_block_snapshots_is_not_stamped_and_the_next_boot_migrates() {
    // Hardening Phase 2: the same incident shape as 0007's, for 0002. A
    // data dir whose `migration_agent_zones_v1.flag` says done while an
    // agent block still holds a snapshot its agent's `:current` zone never
    // received. Bootstrap must leave 0002 pending; the boot must migrate.
    use crate::backend::agent_session::{agent_current_zone, MIGRATION_MARKER_V1};
    use crate::backend::obj::{Block, MetaMapType};
    use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

    let home = TempHome::new();
    home.mark_as_existing_install();
    let wstore = Store::open(&home.channel_store()).unwrap();
    let filestore_path = home.data_dir().join("db").join("filestore.db");
    let filestore = FileStore::open(&filestore_path).unwrap();
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    meta.insert("agentId".to_string(), serde_json::json!("def-stale"));
    let mut block = Block {
        oid: "block-stale".to_string(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    wstore.insert(&mut block).unwrap();
    filestore.make_file("block-stale", "output.state.json", FileMeta::default(), FileOpts::default()).unwrap();
    filestore.write_file("block-stale", "output.state.json", br#"{"nodes":[{"type":"user_message","message":"orphaned"}]}"#).unwrap();
    drop((wstore, filestore));
    std::fs::write(home.data_dir().join(MIGRATION_MARKER_V1), b"v1\n").unwrap();

    M0000Bootstrap.up(&home.ctx()).unwrap();
    assert!(
        !home.applied(MigrationScope::Channel).contains(&"0002_block_zones_v1".to_string()),
        "a stale zones flag must not be stamped as applied"
    );

    home.apply_all(None);
    assert!(home.applied(MigrationScope::Channel).contains(&"0002_block_zones_v1".to_string()));
    let filestore = FileStore::open(&filestore_path).unwrap();
    let current = filestore.read_file(&agent_current_zone("def-stale"), "output.state.json").unwrap();
    assert!(current.is_some_and(|b| String::from_utf8_lossy(&b).contains("orphaned")), "the boot migrated the orphaned snapshot");
}

// ── 2. Crash between effect and mark ──────────────────────────────────────────

#[test]
fn a_crash_between_a_migrations_effect_and_its_mark_resumes_without_duplicating_rows() {
    // agents_consolidate.rs claims (in a comment) that losing the
    // `db_migrations` row after the backfill committed is safe: the next
    // boot re-runs 0007, its content-checked marker short-circuits, and
    // nothing is written twice. This is that claim as a test.
    let home = TempHome::new();
    home.mark_as_existing_install();
    home.insert_legacy_definition("tpl-1", "Coder");
    home.insert_legacy_definition("tpl-2", "Reviewer");

    // A memory file where the fixture's CLAUDE_CONFIG_DIR says it should be,
    // so the full-registry boot below also proves m0024 reads from INSIDE
    // the temp home (it imports the file) rather than from the real one.
    let memory_dir = home.agent_memory_dir("tpl-1");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("note.md"), "# remembered\n").unwrap();

    let first = home.apply_all(None);
    assert_eq!(first.skipped, 0);
    assert_eq!(count(&home.channel_store(), "db_agents"), 2);
    assert!(home.data_dir().join(CONSOLIDATE_FLAG).exists(), "0007 wrote its own marker");
    let imported = Store::open_shared(&home.shared_store())
        .unwrap()
        .agent_native_memory_version_latest("tpl-1", "note.md")
        .unwrap();
    assert!(
        imported.is_some_and(|v| v.content == "# remembered\n"),
        "m0024 must import the memory file from the temp home's providers dir"
    );

    // Simulate the crash in the window the spec names — AFTER the backfill
    // transaction committed, BEFORE the marker was written (and so before
    // the runner could record the row either). The data is there; nothing
    // says so. `agents_consolidate.rs` documents this as "next start retries
    // from scratch" — which is only safe if a from-scratch retry over
    // already-consolidated rows writes nothing twice (codex P2 on #3066:
    // deleting only the tracking row would have left the marker to
    // short-circuit the re-run and proved nothing about that).
    //
    // This simulation runs later than the crash it's modeling actually
    // could: `first` above already ran the REAL, full registry to
    // completion, including `m0029_drop_legacy_agent_tables` — which is
    // registered after `0007` and physically drops
    // `db_agent_definitions`/`db_agent_instances` once nothing needs them
    // any more (agent-concept consolidation Phase 3e,
    // `SPEC_AGENT_ARCHITECTURE_2026_05_27.md`). A REAL crash between 0007's
    // commit and its marker write happens mid-pass, before the pass ever
    // reaches 0029 — so on the real next boot, 0007 retries while its
    // source tables still exist, same as the assertions below used to
    // assume unconditionally. Forcing the retry here, AFTER 0029 has
    // already run, tests the harder edge of the same claim: even with the
    // source gone, the retry must still not duplicate `db_agents` rows —
    // it just can't rewrite a marker for a backfill that no longer has
    // anything left to read.
    std::fs::remove_file(home.data_dir().join(CONSOLIDATE_FLAG)).unwrap();
    Connection::open(home.channel_store())
        .unwrap()
        .execute("DELETE FROM db_migrations WHERE id = ?1", [CONSOLIDATE_ID])
        .unwrap();
    assert!(!home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()));

    let second = home.apply_all(None);
    assert_eq!(second.applied, 1, "exactly the unmarked migration re-runs");
    assert_eq!(second.skipped, REGISTRY.len() - 1);
    assert_eq!(count(&home.channel_store(), "db_agents"), 2, "a from-scratch retry must not duplicate rows");
    // NOT re-written: db_agent_definitions is gone (dropped by 0029 during
    // `first`), so `run_consolidate_migration` correctly finds nothing to
    // consolidate and returns before ever reaching the marker-write step —
    // the same "no source table" tolerance that lets 0007 run harmlessly
    // on a channel that never had the legacy tables at all (see
    // `agents_consolidate::consolidate_looks_incomplete`'s doc comment).
    assert!(!home.data_dir().join(CONSOLIDATE_FLAG).exists(), "nothing left to consolidate — the marker stays absent, not stale");
    assert!(home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()), "0007 is still marked applied — up() returning Ok(()) with nothing to do is success, same as every other migration's exists()-guard no-op");

    // Third boot: nothing to do.
    assert_eq!(home.apply_all(None).applied, 0);
}

#[test]
fn losing_only_the_tracking_row_after_the_marker_is_written_short_circuits_on_the_next_boot() {
    // The narrower window: marker on disk, tracking row lost (a crash between
    // the marker write and `migration_mark_applied`). The content-checked
    // marker short-circuits the re-run (`already_done`), so this is the cheap
    // path — kept as its own test so a regression in either window is named.
    let home = TempHome::new();
    home.mark_as_existing_install();
    home.insert_legacy_definition("tpl-1", "Coder");
    home.apply_all(None);
    assert_eq!(count(&home.channel_store(), "db_agents"), 1);

    Connection::open(home.channel_store())
        .unwrap()
        .execute("DELETE FROM db_migrations WHERE id = ?1", [CONSOLIDATE_ID])
        .unwrap();

    let second = home.apply_all(None);
    assert_eq!(second.applied, 1);
    assert_eq!(count(&home.channel_store(), "db_agents"), 1);
    assert!(home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()));
}

// ── 3. Upgrade skipping many versions ─────────────────────────────────────────

#[test]
fn an_upgrade_that_skips_many_versions_applies_every_later_migration_once_in_registry_order() {
    // A `db_migrations` that stops at 0005 — an install last run on a build
    // from before 0006 existed — must get 0006 through the end of the
    // registry, each exactly once, in order.
    let home = TempHome::new();
    home.mark_as_existing_install();
    let shared = Store::open_shared(&home.shared_store()).unwrap();
    let channel = Store::open(&home.channel_store()).unwrap();
    let ids = registry_ids();
    let (already, expected): (Vec<&str>, Vec<&str>) = (ids[..6].to_vec(), ids[6..].to_vec());
    for id in &already {
        match scope_of(id) {
            MigrationScope::Global => shared.migration_mark_applied(id, "global", 0).unwrap(),
            MigrationScope::Channel => channel.migration_mark_applied(id, "channel", 0).unwrap(),
        };
    }
    drop((shared, channel));

    let order = RefCell::new(Vec::<String>::new());
    let progress = ApplyProgress {
        on_start: &|_, _| {},
        on_done: &|id, _| order.borrow_mut().push(id.to_string()),
    };
    let outcome = home.apply_all(Some(&progress));

    assert_eq!(outcome.applied, expected.len());
    assert_eq!(outcome.skipped, already.len());
    assert_eq!(order.borrow().iter().map(String::as_str).collect::<Vec<_>>(), expected, "registry order, no gaps");

    // Every id is now recorded in its own scope's store, and a further boot
    // is a no-op.
    for id in &ids {
        assert!(home.applied(scope_of(id)).contains(&id.to_string()), "{id} recorded");
    }
    assert_eq!(home.apply_all(None).applied, 0);
}
