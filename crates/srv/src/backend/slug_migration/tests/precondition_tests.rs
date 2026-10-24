// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The preconditions refuse before anything is written: a running agent, an
//! AgentMux that doesn't understand former slugs, a migration already under
//! way.

use super::super::stamp::{self, Stamp};
use crate::backend::rpc_types::ForkSlugBlockerKind;
use super::super::run;
use super::fixture::*;

fn fixture() -> (Fx, std::path::PathBuf) {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    (fx, data)
}

fn refuses_and_writes_nothing(fx: &Fx, what: &str) -> String {
    let before = fx.snapshot();
    let err = super::apply(&fx.home).unwrap_err();
    assert_eq!(fx.snapshot(), before, "{what}: nothing written");
    assert!(!fx.home.backups().exists(), "{what}: no backup taken");
    err
}

#[test]
fn a_running_agent_refuses() {
    let (fx, _) = fixture();
    let lease = serde_json::json!({
        "instance_id": LOAP2, "owner_boot_id": "boot", "block_id": "block-1", "session_id": null,
        "pid": std::process::id(), "acquired_at_ms": 1, "renewed_at_ms": agentmux_common::time::now_ms(),
        "hostname": whoami::fallible::hostname().unwrap_or_default().trim().to_lowercase(), "channel": "stable"
    });
    std::fs::create_dir_all(fx.home.leases()).unwrap();
    std::fs::write(fx.home.leases().join(format!("{LOAP2}.lease.json")), serde_json::to_vec(&lease).unwrap()).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert_eq!(plan.candidates.iter().find(|c| c.uid == LOAP2).unwrap().running[0].block_id, "block-1");
    let err = refuses_and_writes_nothing(&fx, "running");
    assert!(err.contains("Loap #2 is running"), "{err}");

    // A lease its holder stopped renewing doesn't count.
    let mut stale = lease.clone();
    stale["renewed_at_ms"] = 1.into();
    std::fs::write(fx.home.leases().join(format!("{LOAP2}.lease.json")), serde_json::to_vec(&stale).unwrap()).unwrap();
    assert!(super::super::plan(&fx.home, false).unwrap().blockers.is_empty());
}

#[test]
fn a_live_registration_refuses() {
    let (fx, _) = fixture();
    let entry = serde_json::json!({ "agent_id": "naki-naki-2", "local_url": "", "block_id": "b", "pid": std::process::id(), "updated_at": 1, "channel": "local-x", "uid": NAKI2 });
    let dir = fx.home.reactive().join("naki-naki-2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("local-x.json"), serde_json::to_vec(&entry).unwrap()).unwrap();
    let err = refuses_and_writes_nothing(&fx, "registered");
    assert!(err.contains("Naki #2 is running"), "{err}");
}

#[test]
fn an_old_version_instance_refuses() {
    let (fx, data) = fixture();
    super::super::preconditions::test_srv::treat_as_srv(&[std::process::id()]);
    std::fs::write(data.join("srv.lock"), std::process::id().to_string()).unwrap();
    let err = refuses_and_writes_nothing(&fx, "no stamp");
    assert!(err.contains("older AgentMux is running"), "{err}");

    let without = Stamp { pid: std::process::id(), version: "0.59.37".into(), features: vec![] };
    std::fs::write(data.join(stamp::STAMP_FILE), serde_json::to_vec(&without).unwrap()).unwrap();
    let err = refuses_and_writes_nothing(&fx, "no feature");
    assert!(err.contains("0.59.37") && err.contains("doesn't know former slugs"), "{err}");

    let other_pid = Stamp { pid: u32::MAX, version: "0.60.0".into(), features: vec![stamp::FORMER_SLUGS.into()] };
    std::fs::write(data.join(stamp::STAMP_FILE), serde_json::to_vec(&other_pid).unwrap()).unwrap();
    refuses_and_writes_nothing(&fx, "a stamp left by another srv");

    stamp::write(&data);
    assert_eq!(stamp::read(&data.join(stamp::STAMP_FILE)).map(|s| s.has(stamp::FORMER_SLUGS)), Some(true));
    assert!(super::apply(&fx.home).is_ok(), "this build's own stamp is accepted");
}

#[test]
fn an_instance_known_only_to_the_registry_refuses() {
    let (fx, _) = fixture();
    super::super::preconditions::test_srv::treat_as_srv(&[std::process::id()]);
    let entry = serde_json::json!({ "agent_id": "someone", "local_url": "", "block_id": "b", "pid": std::process::id(), "updated_at": 1, "channel": "ancient" });
    let dir = fx.home.reactive().join("someone");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ancient.json"), serde_json::to_vec(&entry).unwrap()).unwrap();
    let err = refuses_and_writes_nothing(&fx, "unstamped registry instance");
    assert!(err.contains("channel ancient"), "{err}");
}

#[test]
fn a_held_lock_refuses() {
    let (fx, _) = fixture();
    let held = super::super::preconditions::take_lock(&fx.home).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(
        plan.blockers.iter().any(|b| b.kind == ForkSlugBlockerKind::Locked && !b.clearable_by_stopping && b.message.contains("under way")),
        "{:?}",
        plan.blockers
    );
    let err = super::apply(&fx.home).unwrap_err();
    assert!(err.contains("under way"), "{err}");
    assert!(super::super::run::rollback(&fx.home, None).unwrap_err().contains("under way"));
    drop(held);
    assert!(super::apply(&fx.home).is_ok());
}

/// Each blocker says what it is, and only a running agent is one that
/// stopping its pane clears: an older AgentMux beside it is not, so the card
/// never stops a pane for a run that still couldn't start.
#[test]
fn blockers_say_their_kind_and_only_a_running_agent_is_cleared_by_stopping() {
    let (fx, data) = fixture();
    super::super::preconditions::test_srv::treat_as_srv(&[std::process::id()]);
    let entry = serde_json::json!({ "agent_id": "naki-naki-2", "local_url": "", "block_id": "b", "pid": std::process::id(), "updated_at": 1, "channel": "stable", "uid": NAKI2 });
    let dir = fx.home.reactive().join("naki-naki-2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stable.json"), serde_json::to_vec(&entry).unwrap()).unwrap();
    stamp::write(&data);
    std::fs::write(data.join("srv.lock"), std::process::id().to_string()).unwrap();
    let running_only = super::super::plan(&fx.home, false).unwrap().blockers;
    assert!(!running_only.is_empty());
    assert!(running_only.iter().all(|b| b.kind == ForkSlugBlockerKind::Running && b.clearable_by_stopping), "{running_only:?}");

    std::fs::remove_file(data.join(stamp::STAMP_FILE)).unwrap();
    let both = super::super::plan(&fx.home, false).unwrap().blockers;
    let old: Vec<_> = both.iter().filter(|b| b.kind == ForkSlugBlockerKind::OldInstance).collect();
    assert_eq!(old.len(), 1, "{both:?}");
    assert!(!old[0].clearable_by_stopping);
    assert!(old[0].message.starts_with("An older AgentMux is running: channels/stable@0.59.1"), "{}", old[0].message);
    assert!(old[0].message.ends_with("Update or quit it first."), "{}", old[0].message);
    assert!(both.iter().any(|b| b.kind == ForkSlugBlockerKind::Running && b.clearable_by_stopping));
}

/// A live lease for `uid`, as a pane running it holds.
fn lease(fx: &Fx, uid: &str) {
    let lease = serde_json::json!({
        "instance_id": uid, "owner_boot_id": "boot", "block_id": "block-1", "session_id": null,
        "pid": std::process::id(), "acquired_at_ms": 1, "renewed_at_ms": agentmux_common::time::now_ms(),
        "hostname": whoami::fallible::hostname().unwrap_or_default().trim().to_lowercase(), "channel": "stable"
    });
    std::fs::create_dir_all(fx.home.leases()).unwrap();
    std::fs::write(fx.home.leases().join(format!("{uid}.lease.json")), serde_json::to_vec(&lease).unwrap()).unwrap();
}

/// Neither way back runs while a migrated agent runs again, under its new
/// id: the undo would rename it from under its process. Nothing changes,
/// the journal included.
#[test]
fn a_rollback_or_restore_refuses_while_a_migrated_agent_runs() {
    let fx = Fx::standard();
    let report = super::apply(&fx.home).unwrap();
    lease(&fx, LOAP2);
    let journal = std::path::Path::new(&report.journal);
    let records = std::fs::read(journal).unwrap();
    let before = fx.snapshot();
    for err in [run::rollback(&fx.home, Some(&report.journal)).unwrap_err(), run::restore(&fx.home, Some(&report.journal)).unwrap_err()] {
        assert!(err.starts_with("not rolling back") && err.contains("Loap #2 is running"), "{err}");
    }
    assert_eq!(fx.snapshot(), before);
    assert_eq!(std::fs::read(journal).unwrap(), records, "the journal is unchanged");

    // A registration under the new id refuses too.
    std::fs::remove_file(fx.home.leases().join(format!("{LOAP2}.lease.json"))).unwrap();
    let entry = serde_json::json!({ "agent_id": "loap-2", "local_url": "", "block_id": "b", "pid": std::process::id(), "updated_at": 1, "channel": "stable", "uid": LOAP2 });
    let dir = fx.home.reactive().join("loap-2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stable.json"), serde_json::to_vec(&entry).unwrap()).unwrap();
    assert!(run::rollback(&fx.home, Some(&report.journal)).unwrap_err().contains("Loap #2 is running"));
    std::fs::remove_dir_all(&dir).unwrap();
    run::rollback(&fx.home, Some(&report.journal)).unwrap();
}

/// A corrupt `objects.db` anywhere blocks the plan, naming the file, and no
/// target reads as free: every candidate waits and an apply writes nothing.
#[test]
fn a_corrupt_objects_db_blocks_the_plan() {
    let (fx, _) = fixture();
    let bad = fx.root().join("channels/stable/versions/0.50.0/data/db");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("objects.db"), vec![0x5au8; 8192]).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(plan.candidates.iter().all(|c| c.refused.is_some()), "{:?}", plan.candidates);
    let blocker = plan.blockers.iter().find(|b| b.message.contains("0.50.0")).unwrap_or_else(|| panic!("{:?}", plan.blockers));
    assert_eq!(blocker.kind, ForkSlugBlockerKind::Unreadable);
    assert!(!blocker.clearable_by_stopping);
    let err = super::apply(&fx.home).unwrap_err();
    assert!(err.contains("can't be read"), "{err}");
    assert!(!fx.home.backups().exists(), "nothing written");
}

/// A stale srv.lock whose pid another program now has (chrome.exe, on the
/// operator's home) isn't an older AgentMux running (#1270).
#[test]
fn a_stale_lock_whose_pid_was_reused_doesnt_block() {
    let (fx, data) = fixture();
    // This process is alive but isn't an srv: the pid a stale lock names.
    super::super::preconditions::test_srv::treat_as_srv(&[]);
    std::fs::write(data.join("srv.lock"), std::process::id().to_string()).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(plan.blockers.iter().all(|b| b.kind != ForkSlugBlockerKind::OldInstance), "{:?}", plan.blockers);
    assert!(super::apply(&fx.home).is_ok());
}

#[test]
fn only_an_srv_started_before_its_lock_was_written_holds_it() {
    use super::super::preconditions::is_srv_that_wrote;
    assert!(is_srv_that_wrote("agentmux-srv.exe", 100, Some(100)));
    assert!(is_srv_that_wrote("agentmux-srv-0.59.40-linux.x64", 100, Some(103)), "Linux names its binary by version");
    assert!(is_srv_that_wrote("AgentMux-Srv.exe", 100, None), "no lock time: the name decides");
    assert!(!is_srv_that_wrote("chrome.exe", 100, Some(200)));
    assert!(!is_srv_that_wrote("agentmux_srv-1a2b3c.exe", 100, Some(200)), "a test binary isn't one");
    assert!(!is_srv_that_wrote("agentmux-srv.exe", 500, Some(100)), "started after the lock was written: a reused pid");
}

/// The real lookup reads this process's name: a test binary, so not an srv.
#[test]
fn this_test_process_is_not_taken_for_an_srv() {
    let target = sysinfo::Pid::from(std::process::id() as usize);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[target]), true, sysinfo::ProcessRefreshKind::nothing());
    let name = sys.process(target).map(|p| p.name().to_string_lossy().into_owned()).unwrap_or_default();
    assert!(!name.is_empty(), "the name is read without a full refresh");
    assert!(!super::super::preconditions::is_srv_that_wrote(&name, 0, None), "{name}");
}
