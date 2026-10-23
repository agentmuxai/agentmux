// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The fixture home end to end: plan, apply, verify; then roll back and
//! compare every digest to the original; and restore mode.

use super::super::run;
use super::fixture::*;

#[test]
fn the_fixture_home_migrates_verifies_and_rolls_back_to_its_checksums() {
    let fx = Fx::standard();
    let original = fx.snapshot();
    let report = super::apply(&fx.home).unwrap();
    assert_eq!(report.failed, None);
    assert_eq!(report.migrated.len(), 3);
    assert_eq!(
        report.pr_searches.iter().filter(|s| s.contains("agentmux:agent_id=loap-loap-2 in:body")).count(),
        1,
        "{:?}",
        report.pr_searches
    );
    assert!(std::path::Path::new(&report.journal).is_file());

    // The new slug everywhere it was stored, and the old one only as a former slug.
    for (uid, new) in [(LOAP2, "loap-2"), (LOAP3, "loap-3"), (NAKI2, "naki-2")] {
        assert_eq!(read_json(&fx.home.definitions().join(format!("{uid}.json")))["data"]["slug"], new);
        for install in fx.home.installs().unwrap() {
            for v in &install.versions {
                assert_eq!(query::<String>(&v.objects_db(), &format!("SELECT slug FROM db_agents WHERE id = '{uid}'")).as_deref(), Some(new));
            }
        }
    }
    let stable = fx.root().join("channels/stable");
    assert!(stable.join("config/gh-loap-2").is_dir() && !stable.join("config/gh-loap-loap-2").exists());
    assert!(!stable.join("versions/0.59.1/data/agents/loap-loap-2.json").exists());
    assert!(!fx.home.reactive().join("loap-loap-2").exists());
    assert_eq!(query::<String>(&stable.join("wan-identity/wan.db"), "SELECT agent_id FROM wan_agent_keys WHERE agent_id = 'loap-loap-2'"), None);
    assert_eq!(
        query::<String>(&fx.home.shared.join("store.db"), "SELECT client_id FROM db_agent_credentials WHERE agent_id = 'loap-loap-2'").as_deref(),
        Some("cid"),
        "S5 keeps the old name's MuxBus credential"
    );

    // The planner is idempotent: a migrated agent is no longer a candidate.
    assert!(super::super::plan(&fx.home, false).unwrap().candidates.is_empty());
    // A second apply finds nothing to do.
    assert!(super::apply(&fx.home).unwrap().migrated.is_empty());

    run::rollback(&fx.home, Some(&report.journal)).unwrap();
    let back = fx.snapshot();
    let diff: Vec<_> = original.keys().chain(back.keys()).filter(|k| original.get(*k) != back.get(*k)).collect();
    assert!(diff.is_empty(), "after the rollback these differ from the original: {diff:?}");
}

#[test]
fn restore_mode_puts_the_backup_back_only_while_nothing_changed() {
    let fx = Fx::standard();
    let original = fx.snapshot();
    let report = super::apply(&fx.home).unwrap();
    run::restore(&fx.home, Some(&report.journal)).unwrap();
    assert_eq!(fx.snapshot(), original);

    // A database written since the run keeps that write (#5342).
    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    let db = fx.root().join("channels/stable/versions/0.59.2/data/db/objects.db");
    fx.exec(&db, "UPDATE db_agents SET description = 'edited since' WHERE id = 'uid-naki'");
    run::restore(&fx.home, Some(&report.journal)).unwrap();
    assert_eq!(query::<String>(&db, "SELECT description FROM db_agents WHERE id = 'uid-naki'").as_deref(), Some("edited since"));
    for v in ["0.59.1", "0.59.2"] {
        let db = fx.root().join(format!("channels/stable/versions/{v}/data/db/objects.db"));
        assert_eq!(query::<String>(&db, "SELECT slug FROM db_agents WHERE id = 'uid-loap-2'").as_deref(), Some("loap-loap-2"), "{v}");
    }

    // A file written since the run can't be put back from the backup.
    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    let def = fx.home.definitions().join("uid-loap-2.json");
    let mut bytes = std::fs::read(&def).unwrap();
    bytes.extend_from_slice(b" ");
    std::fs::write(&def, &bytes).unwrap();
    let err = run::restore(&fx.home, Some(&report.journal)).unwrap_err();
    assert!(err.contains("written since the run"), "{err}");
    run::rollback(&fx.home, Some(&report.journal)).unwrap();
}

/// Writes the fixture home to `SLUG_MIGRATION_FIXTURE_OUT`, for a live run of
/// the real binary against a copy of it (`agentmux-srv migrate-fork-slugs`
/// with `AGENTMUX_HOME_OVERRIDE`). Run by hand: `cargo test -p agentmux-srv
/// write_the_fixture_home -- --ignored`.
#[test]
#[ignore]
fn write_the_fixture_home() {
    let Some(out) = std::env::var_os("SLUG_MIGRATION_FIXTURE_OUT") else { return };
    let fx = Fx::standard();
    copy_tree(fx.root(), std::path::Path::new(&out));
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The backup holds a `VACUUM INTO` copy of every database and the files a
/// run changes, each matching the checksum the journal recorded.
#[test]
fn the_backup_copies_every_database_with_its_checksum() {
    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    let journal = super::super::journal::Journal::open(std::path::Path::new(&report.journal)).unwrap();
    let backups = journal.header().2;
    let dbs = backups.iter().filter(|b| b.database).count();
    assert_eq!(dbs, 4 + 2 + 2, "four objects.db, two wan.db, store.db and identity-store.db");
    for b in backups {
        let copy = super::super::backup::copy_of(&journal.dir(), backups, std::path::Path::new(&b.original)).unwrap();
        if b.database {
            rusqlite::Connection::open(&copy).unwrap().query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap();
        }
    }
    assert!(backups.iter().any(|b| b.original.ends_with(&format!("{LOAP2}.json"))));
    assert!(backups.iter().any(|b| b.original.ends_with(".mcp.json")));
}

/// Another channel creates an agent with a planned target after the plan
/// (during the backup, say): under the slug allocation lock the run checks
/// the target again before S1, leaves that agent out with nothing written
/// for it, and migrates the others.
#[test]
fn a_target_taken_after_the_plan_leaves_that_agent_out_untouched() {
    let fx = Fx::standard();
    let stable_a = fx.root().join("channels/stable/versions/0.59.1/data");
    let def_before = std::fs::read(fx.home.definitions().join(format!("{LOAP2}.json"))).unwrap();
    let defs = fx.defs.clone();
    let db = stable_a.join("db/objects.db");
    let done = std::sync::atomic::AtomicBool::new(false);
    let opts = run::Options {
        before_recheck: Some(Box::new(move || {
            if done.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let store = crate::backend::storage::store::Store::open(&db).unwrap();
            store.set_def_registry(defs.clone());
            let mut def = crate::backend::storage::agents::test_agent_def("uid-new", "Newcomer", "claude", "host", 2_000, "");
            def.slug = "loap-2".into();
            store.agent_def_insert(&mut def).unwrap();
            assert_eq!(store.agent_def_get("uid-new").unwrap().unwrap().slug, "loap-2", "the newcomer got the planned id");
        })),
        ..Default::default()
    };
    let report = run::apply(&fx.home, &opts).unwrap();
    let mut migrated: Vec<&str> = report.migrated.iter().map(|m| m.new_slug.as_str()).collect();
    migrated.sort();
    assert_eq!(migrated, vec!["loap-3", "naki-2"]);
    assert!(report.notes.iter().any(|n| n.contains("Loap #2 wasn't updated") && n.contains("uid-new")), "{:?}", report.notes);
    assert_eq!(std::fs::read(fx.home.definitions().join(format!("{LOAP2}.json"))).unwrap(), def_before, "its definition is untouched");
    for install in fx.home.installs().unwrap() {
        for v in &install.versions {
            assert_eq!(query::<String>(&v.objects_db(), &format!("SELECT slug FROM db_agents WHERE id = '{LOAP2}'")).as_deref(), Some("loap-loap-2"));
        }
    }
    assert_eq!(
        query::<String>(&stable_a.join("db/objects.db"), "SELECT hmac_key FROM db_agent_jekt_keys WHERE agent_id = 'loap-loap-2'").as_deref(),
        Some("hmac-loap-loap-2"),
        "its keys are untouched"
    );
    let journal = super::super::journal::Journal::open(std::path::Path::new(&report.journal)).unwrap();
    assert!(journal.intent(LOAP2, super::super::journal::Step::S1).is_none(), "no step of it was even begun");
}

/// A run interrupted after S1's intent but before its write holds no
/// reservation: the resume checks again and stops for a rollback (#5342).
#[test]
fn a_resume_checks_the_target_again_until_s1_is_done() {
    let fx = Fx::standard();
    let def = fx.home.definitions().join(format!("{LOAP2}.json"));
    let def_before = std::fs::read(&def).unwrap();
    let opts = run::Options { crash: Some((LOAP2.to_string(), run::Crash::BeforeDone(super::super::journal::Step::S1))), ..Default::default() };
    assert_eq!(run::apply(&fx.home, &opts).unwrap_err(), run::CRASHED);
    // As if S1's write never landed, and another channel then took the id.
    std::fs::write(&def, &def_before).unwrap();
    let db = fx.root().join("channels/stable/versions/0.59.1/data/db/objects.db");
    let store = crate::backend::storage::store::Store::open(&db).unwrap();
    store.set_def_registry(fx.defs.clone());
    let mut newcomer = crate::backend::storage::agents::test_agent_def("uid-new", "Newcomer", "claude", "host", 2_000, "");
    newcomer.slug = "loap-2".into();
    store.agent_def_insert(&mut newcomer).unwrap();
    drop(store);
    let err = run::apply(&fx.home, &run::Options::default()).unwrap_err();
    assert!(err.contains("was taken while the run was interrupted") && err.contains("uid-new"), "{err}");
    run::rollback(&fx.home, None).unwrap();
    assert_eq!(std::fs::read(&def).unwrap(), def_before);
}
