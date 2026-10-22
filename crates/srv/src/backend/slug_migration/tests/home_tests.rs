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

    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    let db = fx.root().join("channels/stable/versions/0.59.2/data/db/objects.db");
    fx.exec(&db, "UPDATE db_agents SET description = 'edited since' WHERE id = 'uid-naki'");
    let err = run::restore(&fx.home, Some(&report.journal)).unwrap_err();
    assert!(err.contains("written since the run"), "{err}");
    run::rollback(&fx.home, Some(&report.journal)).unwrap();
    assert_eq!(query::<String>(&db, "SELECT description FROM db_agents WHERE id = 'uid-naki'").as_deref(), Some("edited since"), "journal mode keeps later edits");
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
