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

/// A legacy `db_agent_definitions` row — the pre-consolidation shape `0007`
/// exists to fold into `db_agents`. Same columns the consolidate module's
/// own tests insert.
fn insert_legacy_definition(channel_store: &Path, id: &str, name: &str) {
    // `Store::open` lays down the schema (including the legacy tables it still
    // adopts); the row itself goes in through SQL because no live API writes
    // that table any more.
    drop(Store::open(channel_store).expect("create channel store"));
    let conn = Connection::open(channel_store).unwrap();
    conn.execute(
        "INSERT INTO db_agent_definitions
            (id, slug, name, icon, provider, description, working_directory, shell,
             provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
             created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
             parent_id, branch_label, updated_at)
         VALUES (?1, ?1, ?2, '✦', 'claude', 'desc', '', 'bash',
                 '', 0, 0, 0,
                 1000, 'standalone', '', '', 1, '',
                 '', '', 1000)",
        params![id, name],
    )
    .unwrap();
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
    insert_legacy_definition(&home.channel_store(), "tpl-1", "Coder");
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
    insert_legacy_definition(&home.channel_store(), "tpl-1", "Coder");
    insert_legacy_definition(&home.channel_store(), "tpl-2", "Reviewer");
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

// ── 2. Crash between effect and mark ──────────────────────────────────────────

#[test]
fn a_crash_between_a_migrations_effect_and_its_mark_resumes_without_duplicating_rows() {
    // agents_consolidate.rs claims (in a comment) that losing the
    // `db_migrations` row after the backfill committed is safe: the next
    // boot re-runs 0007, its content-checked marker short-circuits, and
    // nothing is written twice. This is that claim as a test.
    let home = TempHome::new();
    home.mark_as_existing_install();
    insert_legacy_definition(&home.channel_store(), "tpl-1", "Coder");
    insert_legacy_definition(&home.channel_store(), "tpl-2", "Reviewer");

    let first = home.apply_all(None);
    assert_eq!(first.skipped, 0);
    assert_eq!(count(&home.channel_store(), "db_agents"), 2);
    assert!(home.data_dir().join(CONSOLIDATE_FLAG).exists(), "0007 wrote its own marker");

    // Simulate the crash: the effect and the marker file are on disk, the
    // tracking row never made it.
    Connection::open(home.channel_store())
        .unwrap()
        .execute("DELETE FROM db_migrations WHERE id = ?1", [CONSOLIDATE_ID])
        .unwrap();
    assert!(!home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()));

    let second = home.apply_all(None);
    assert_eq!(second.applied, 1, "exactly the unmarked migration re-runs");
    assert_eq!(second.skipped, REGISTRY.len() - 1);
    assert_eq!(count(&home.channel_store(), "db_agents"), 2, "the re-run must not duplicate rows");
    assert!(home.applied(MigrationScope::Channel).contains(&CONSOLIDATE_ID.to_string()), "and it is marked again");

    // Third boot: nothing to do.
    assert_eq!(home.apply_all(None).applied, 0);
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
