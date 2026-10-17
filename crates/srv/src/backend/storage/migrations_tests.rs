// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for `migrations.rs`, kept beside it so the module stays readable.

use super::*;

/// Every table the flat `objects.db` schema must contain.
const EXPECTED_TABLES: &[&str] = &[
    "db_client",
    "db_window",
    "db_workspace",
    "db_tab",
    "db_layout",
    "db_block",
    "db_temp",
    "db_agent_content",
    "db_agent_skills",
    "db_agent_history",
    "db_accounts",
    "db_agent_identity_links",
    "db_bundles",
    "db_agents",
    "db_drone_definitions",
    "db_drone_runs",
];

fn table_exists(conn: &Connection, name: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap();
    count == 1
}

fn index_exists(conn: &Connection, name: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap();
    count == 1
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    names.iter().any(|n| n == column)
}

#[test]
fn test_object_schema_creates_all_tables_and_singletons() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&conn).unwrap();

    for table in EXPECTED_TABLES {
        assert!(table_exists(&conn, table), "{table} should exist");
    }
    // De-forged + bundle indexes.
    for idx in &[
        "idx_agent_history_agent_date",
        "idx_agent_identity_links_account",
        "idx_accounts_provider",
        "idx_bundles_is_blank",
        "idx_agents_is_template",
        "idx_agents_parent_template_id",
        "idx_agents_is_seeded",
        "idx_drone_definitions_updated",
        "idx_drone_runs_status",
    ] {
        assert!(index_exists(&conn, idx), "{idx} should exist");
    }

    // Blank singleton seeded.
    let mem_blank: i64 = conn
        .query_row(
            "SELECT count(*) FROM db_bundles WHERE id='blank' AND is_blank=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mem_blank, 1, "blank Bundle singleton should be seeded");
}

#[test]
fn test_object_schema_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&conn).unwrap();
    run_object_schema(&conn).unwrap(); // second pass must not error

    // Singleton stays unique.
    let mem_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM db_bundles WHERE id='blank'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mem_count, 1);
}

#[test]
fn test_object_schema_has_model_vendor_base_url_on_db_agents() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&conn).unwrap();
    // Second pass: the ALTER TABLE array's duplicate-column guard must
    // not error when the column already exists (fresh installs get it
    // via CREATE TABLE; the ALTER is for upgrading pre-v15 databases).
    // Was "on_both_tables" — db_agent_definitions is gone as of v32, so
    // this is db_agents-only now (see OBJECT_SCHEMA_VERSION's v32 doc
    // comment).
    run_object_schema(&conn).unwrap();

    assert!(column_exists(&conn, "db_agents", "model_vendor_base_url"));
}

#[test]
fn test_object_schema_has_auto_continue_enabled_on_db_agents() {
    // The column is retired (nothing reads or writes it since the Warden
    // Supervisor was removed) but stays in the schema: older builds share
    // the data dir and still select it.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&conn).unwrap();
    // Second pass: the ALTER TABLE array's duplicate-column guard must
    // not error when the column already exists (fresh installs get it
    // via CREATE TABLE; the ALTER is for upgrading pre-v17 databases).
    // Was "on_both_tables" — see the v15 test just above for why.
    run_object_schema(&conn).unwrap();

    assert!(column_exists(&conn, "db_agents", "auto_continue_enabled"));
}

#[test]
fn test_db_bundles_has_instructions_by_provider_in_both_schemas() {
    // ABF v0.2 §2.2 (v16): db_bundles lives in both objects.db (via
    // run_object_schema) and the shared store.db (via
    // run_shared_store_schema) — mirrors the db_agent_credentials /
    // db_agent_native_memory both-schemas precedent (v13/v14).
    let object_conn = Connection::open_in_memory().unwrap();
    object_conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&object_conn).unwrap();
    run_object_schema(&object_conn).unwrap(); // idempotent second pass
    assert!(column_exists(&object_conn, "db_bundles", "instructions_by_provider"));

    let shared_conn = Connection::open_in_memory().unwrap();
    shared_conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_shared_store_schema(&shared_conn).unwrap();
    run_shared_store_schema(&shared_conn).unwrap();
    assert!(column_exists(&shared_conn, "db_bundles", "instructions_by_provider"));
}

#[test]
fn test_db_skills_and_mcp_servers_exist_in_both_object_and_identity_schemas() {
    // Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md (v6): the identity
    // store gains its own db_skills/db_mcp_servers, becoming
    // authoritative — but the channel-store copies (run_object_schema)
    // are NOT removed, per that spec's §5.1/§8 item 3. Pins both halves
    // of that claim, not just the new one: it would be easy to
    // accidentally satisfy only the identity-store half of this and
    // silently drop the channel-store fallback.
    let object_conn = Connection::open_in_memory().unwrap();
    run_object_schema(&object_conn).unwrap();
    assert!(table_exists(&object_conn, "db_skills"), "channel store must keep db_skills");
    assert!(table_exists(&object_conn, "db_mcp_servers"), "channel store must keep db_mcp_servers");

    let identity_conn = Connection::open_in_memory().unwrap();
    run_identity_store_schema(&identity_conn).unwrap();
    run_identity_store_schema(&identity_conn).unwrap(); // idempotent second pass
    assert!(table_exists(&identity_conn, "db_skills"), "identity store must have db_skills");
    assert!(table_exists(&identity_conn, "db_mcp_servers"), "identity store must have db_mcp_servers");
    assert!(index_exists(&identity_conn, "idx_ids_skills_is_global"));
    assert!(index_exists(&identity_conn, "idx_ids_mcp_servers_is_global"));

    // No ref tables here yet — those are Phase 3/6, not Phase 2. Schema
    // for a phase that hasn't shipped would be its own kind of drift.
    assert!(!table_exists(&identity_conn, "db_agent_skills_ref"));
    assert!(!table_exists(&identity_conn, "db_bundle_skills_ref"));
    assert!(!table_exists(&identity_conn, "db_agent_mcp_ref"));
    assert!(!table_exists(&identity_conn, "db_bundle_mcp_ref"));
}

#[test]
fn test_object_schema_omits_dead_tables() {
    let conn = Connection::open_in_memory().unwrap();
    run_object_schema(&conn).unwrap();
    for dead in DEAD_TABLE_DROPS {
        assert!(!table_exists(&conn, dead), "{dead} must not be created");
    }
    // Legacy forge names are never created either.
    for (legacy, _) in LEGACY_TABLE_RENAMES {
        assert!(
            !table_exists(&conn, legacy),
            "legacy {legacy} must not be created by the flat schema"
        );
    }
}

#[test]
fn test_adopt_legacy_renames_forge_tables() {
    // Simulate a pre-flatten (post-v11) dev DB: legacy forge table
    // names + a dead workflow table, with seeded rows.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(
        "CREATE TABLE db_forge_agents (
            id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
            icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
            shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
            auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
            agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
            accounts TEXT NOT NULL DEFAULT '',
            parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX idx_forge_agents_slug ON db_forge_agents(slug);
        INSERT INTO db_forge_agents (id, slug, name, provider)
            VALUES ('a1', 'coder', 'Coder', 'claude');

        CREATE TABLE db_workflow_definitions (id TEXT PRIMARY KEY);",
    )
    .unwrap();

    run_object_schema(&conn).unwrap();

    // Renamed, data preserved.
    assert!(table_exists(&conn, "db_agent_definitions"));
    assert!(!table_exists(&conn, "db_forge_agents"));
    let name: String = conn
        .query_row(
            "SELECT name FROM db_agent_definitions WHERE id='a1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Coder");
    // Old index dropped. No new index takes its place — db_agent_definitions
    // is dropped whole as of v32 (OBJECT_SCHEMA_VERSION's v32 doc comment),
    // so the flat schema no longer declares a unique index for it; the
    // renamed table exists only transiently, until the drop migration
    // that runs later in the same upgrade removes it.
    assert!(!index_exists(&conn, "idx_forge_agents_slug"));
    // Dead table dropped.
    assert!(!table_exists(&conn, "db_workflow_definitions"));
}

#[test]
fn test_adopt_legacy_is_noop_on_fresh_db() {
    let conn = Connection::open_in_memory().unwrap();
    run_object_schema(&conn).unwrap();
    // Re-running schema (which re-runs adopt) on the already-flat DB
    // leaves the de-forged tables intact and creates no legacy names.
    // db_agent_definitions itself is gone from the flat schema as of
    // v32, so a live de-forged table that's still declared (db_agents)
    // stands in for "the flat schema is present, nothing to adopt".
    run_object_schema(&conn).unwrap();
    assert!(table_exists(&conn, "db_agents"));
    assert!(!table_exists(&conn, "db_forge_agents"));
}

#[test]
fn test_adopt_legacy_both_tables_present_is_non_destructive() {
    // Downgrade-roundtrip: a flat DB (db_agent_definitions) where a
    // pre-flatten build later re-created db_forge_agents and wrote a
    // row. The adopt step must NOT drop the legacy table — silent
    // data loss is the bug class behind PR #933's Codex P1.
    let conn = Connection::open_in_memory().unwrap();
    run_object_schema(&conn).unwrap();
    // db_agent_definitions is gone from the flat schema as of v32, so
    // this fixture has to construct the "downgrade-era db already has
    // the de-forged table too" scenario by hand — minimal shape, this
    // test only cares about existence + data survival, not full column
    // parity.
    conn.execute_batch(
        "CREATE TABLE db_agent_definitions (id TEXT PRIMARY KEY, name TEXT NOT NULL);
         CREATE TABLE db_forge_agents (id TEXT PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO db_forge_agents (id, name) VALUES ('downgrade-era', 'Recover Me');",
    )
    .unwrap();

    run_object_schema(&conn).unwrap();

    // Legacy table left intact — data recoverable, not dropped.
    assert!(table_exists(&conn, "db_forge_agents"));
    let name: String = conn
        .query_row(
            "SELECT name FROM db_forge_agents WHERE id='downgrade-era'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Recover Me");
    // Flat table still present and authoritative.
    assert!(table_exists(&conn, "db_agent_definitions"));
}

#[test]
fn test_adopt_legacy_fk_cascade_survives_rename() {
    // A renamed parent must keep cascading into renamed children.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(
        "CREATE TABLE db_forge_agents (
            id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
            icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
            shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
            auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
            agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
            accounts TEXT NOT NULL DEFAULT '',
            parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE db_forge_content (
            agent_id TEXT NOT NULL, content_type TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
        );
        INSERT INTO db_forge_agents (id, name, provider) VALUES ('a1', 'Coder', 'claude');
        INSERT INTO db_forge_content (agent_id, content_type, content)
            VALUES ('a1', 'soul', 'hello');",
    )
    .unwrap();

    run_object_schema(&conn).unwrap();

    conn.execute("DELETE FROM db_agent_definitions WHERE id='a1'", [])
        .unwrap();
    let remaining: i64 = conn
        .query_row(
            "SELECT count(*) FROM db_agent_content WHERE agent_id='a1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "FK cascade must survive the forge→agent table rename"
    );
}

#[test]
fn legacy_column_added_to_a_still_present_db_agent_definitions_via_alter() {
    // Codex P1 on #3096: the flat schema no longer CREATEs
    // db_agent_definitions (dropped as of v32), but several pending
    // migrations (m0007, m0025, m0028) call Store::open() internally —
    // running run_object_schema — before m0029 (numbered last) actually
    // drops the table in the same upgrade pass. A store old enough to
    // still carry the table but missing a column those migrations read
    // (e.g. user_hidden) must still get it ALTERed in, or their own
    // reads fail with "no such column" before they ever reach m0029.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(
        "CREATE TABLE db_agent_definitions (
            id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '✦',
            provider TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            working_directory TEXT NOT NULL DEFAULT '',
            shell TEXT NOT NULL DEFAULT '',
            provider_flags TEXT NOT NULL DEFAULT '',
            auto_start INTEGER NOT NULL DEFAULT 0,
            restart_on_crash INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            agent_type TEXT NOT NULL DEFAULT 'standalone',
            environment TEXT NOT NULL DEFAULT '',
            agent_bus_id TEXT NOT NULL DEFAULT '',
            is_seeded INTEGER NOT NULL DEFAULT 0,
            accounts TEXT NOT NULL DEFAULT '',
            parent_id TEXT NOT NULL DEFAULT '',
            branch_label TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO db_agent_definitions (id, name, provider, is_seeded)
            VALUES ('pre-existing', 'Old Template', 'claude', 1);",
    )
    .unwrap();

    run_object_schema(&conn).unwrap();
    // Idempotent — second pass must not error and must not
    // re-default existing rows.
    run_object_schema(&conn).unwrap();

    let hidden: i64 = conn
        .query_row(
            "SELECT user_hidden FROM db_agent_definitions WHERE id='pre-existing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        hidden, 0,
        "ALTER must default existing rows to 0 (visible), never to 1",
    );
}

#[test]
fn legacy_alter_is_skipped_once_db_agent_definitions_is_gone() {
    // The other half of the gate: once the table doesn't exist at all
    // (a fresh v32+ install, or an upgraded store after m0029 has
    // already dropped it), the conditional ALTER block must not even
    // attempt the statement — "no such table" is not "duplicate
    // column" and would not be swallowed.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    run_object_schema(&conn).unwrap();
    assert!(!table_exists(&conn, "db_agent_definitions"));
}

#[test]
fn stamp_version_writes_pragma() {
    let conn = Connection::open_in_memory().unwrap();
    stamp_version(&conn, OBJECT_SCHEMA_VERSION).unwrap();
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, OBJECT_SCHEMA_VERSION);
}

#[test]
fn check_schema_compat_refuses_newer_db_without_writing() {
    // `SPEC_DATA_CHANNELS_2026_05_24.md` §3.3 safety lock — if the
    // DB on disk was stamped by a newer AgentMux binary, this one
    // MUST refuse to open it. The split into
    // check_schema_compat + stamp_version (codex P1 on #1029)
    // ensures the check runs BEFORE any migration side effects:
    // legacy-table rename + seed-insert in `run_object_schema` are
    // mutating, and if we ran them first we'd partially alter a
    // newer DB before the error fired.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    let err = check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db")
        .expect_err("expected refusal");
    match err {
        StoreError::SchemaTooNew { db, found, expected } => {
            assert_eq!(db, "objects.db");
            assert_eq!(found, 99);
            assert_eq!(expected, OBJECT_SCHEMA_VERSION);
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
    // Crucially, `check_schema_compat` made NO writes. The
    // `user_version` is still 99. (Before the split — when the
    // single function combined the check with the write — this
    // invariant held only by virtue of the early `return Err`
    // skipping the stamp; with the split the function is
    // structurally read-only, eliminating the risk that a
    // future refactor reintroduces a downgrade-corrupt path.)
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 99, "rejected DB must not have its user_version stamp overwritten");
}

#[test]
fn check_schema_compat_accepts_equal_or_lower_without_writing() {
    // check_schema_compat must NEVER write — it just gates
    // migrations. The on-disk `user_version` is untouched by it,
    // regardless of whether the verdict is accept or reject.
    // stamp_version is the only thing that writes, and only after
    // migrations succeed.

    // Equal: check passes silently.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("PRAGMA user_version = {};", OBJECT_SCHEMA_VERSION))
        .unwrap();
    check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, OBJECT_SCHEMA_VERSION);

    // Lower (forward-migration path): check passes, version stays
    // at the OLD value — stamp_version is what bumps it after
    // migrations.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 1;").unwrap();
    check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1, "check_schema_compat must not write to user_version");

    // Then stamp_version bumps it as the post-migration step.
    stamp_version(&conn, OBJECT_SCHEMA_VERSION).unwrap();
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, OBJECT_SCHEMA_VERSION);
}

#[test]
fn test_filestore_migrations_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    run_filestore_migrations(&conn).unwrap();
    run_filestore_migrations(&conn).unwrap();
    assert!(table_exists(&conn, "db_wave_file"));
    assert!(table_exists(&conn, "db_file_data"));
}

#[test]
fn test_saga_log_migrations_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    run_saga_log_migrations(&conn).unwrap();
    run_saga_log_migrations(&conn).unwrap();
    assert!(table_exists(&conn, "saga"));
    assert!(table_exists(&conn, "saga_step"));
}

/// Identity M4c-1: each store's actor-UID columns exist on a fresh
/// store, and an existing store at the previous version gains them on
/// reopen — through each store's real `open`, so the flat CREATE, the
/// ALTER loop and the version bump are all exercised. A row written
/// before the upgrade reads the column's default, `''` (unknown).
#[test]
fn m4c1_actor_uid_columns_on_fresh_and_upgraded_stores() {
    use super::super::store::Store;
    type Open = fn(&std::path::Path) -> Result<Store, StoreError>;
    let cases: [(&str, Open, i64, i64, &[(&str, &str)]); 3] = [
        (
            "identity-store.db",
            Store::open_identity_store,
            10,
            IDENTITY_STORE_SCHEMA_VERSION,
            &[
                ("db_work_queue", "created_by_uid"),
                ("db_cron_jobs", "created_by_uid"),
                ("db_bundle_versions", "written_by_uid"),
            ],
        ),
        (
            "store.db",
            Store::open_shared,
            11,
            SHARED_STORE_SCHEMA_VERSION,
            &[
                ("db_cron_jobs", "created_by_uid"),
                ("db_bundle_versions", "written_by_uid"),
            ],
        ),
        (
            "objects.db",
            Store::open,
            38,
            OBJECT_SCHEMA_VERSION,
            &[("db_bundle_versions", "written_by_uid")],
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (file, open, old, new, cols) in cases {
        // M4c-1 was one bump each; later bumps (objects v40, durable
        // jekt) keep the upgrade from `old` covered.
        assert!(new > old, "{file}: the current version is past M4c-1's base");
        let user_version = |c: &Connection| -> i64 {
            c.query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap()
        };

        // Fresh.
        let fresh = dir.path().join(format!("fresh-{file}"));
        drop(open(&fresh).unwrap());
        let conn = Connection::open(&fresh).unwrap();
        for (table, col) in cols {
            assert!(
                column_exists(&conn, table, col),
                "fresh {file}: {table}.{col}"
            );
        }
        assert_eq!(user_version(&conn), new);
        drop(conn);

        // Upgraded: the same store as the previous version left it —
        // the columns absent, stamped `old` — with a version row in it.
        let path = dir.path().join(file);
        drop(open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        for (table, col) in cols {
            // Rebuilt without the column: SQLite's `DROP COLUMN` fails
            // to re-parse these tables' commented CREATE text.
            let keep: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .map(Result::unwrap)
                .filter(|c| c != col)
                .collect();
            conn.execute_batch(&format!(
                "CREATE TABLE m4c1_old AS SELECT {} FROM {table};
                 DROP TABLE {table};
                 ALTER TABLE m4c1_old RENAME TO {table};",
                keep.join(", ")
            ))
            .unwrap();
            assert!(!column_exists(&conn, table, col));
        }
        conn.execute_batch(&format!(
            "PRAGMA user_version = {old};
             INSERT INTO db_bundle_versions
                 (id, bundle_id, name, instructions, content_hash)
             VALUES ('v-old', 'b', 'n', 'i', 'h');"
        ))
        .unwrap();
        drop(conn);

        drop(open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        for (table, col) in cols {
            assert!(
                column_exists(&conn, table, col),
                "upgraded {file}: {table}.{col}"
            );
        }
        assert_eq!(user_version(&conn), new, "{file}");
        let uid: String = conn
            .query_row(
                "SELECT written_by_uid FROM db_bundle_versions WHERE id = 'v-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            uid, "",
            "{file}: a pre-upgrade row is unknown, never guessed"
        );
    }
}
