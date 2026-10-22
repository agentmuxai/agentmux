// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The preconditions refuse before anything is written: a running agent, an
//! AgentMux that doesn't understand former slugs, a migration already under
//! way.

use super::super::stamp::{self, Stamp};
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
    assert!(plan.blockers.iter().any(|b| b.contains("under way")), "{:?}", plan.blockers);
    let err = super::apply(&fx.home).unwrap_err();
    assert!(err.contains("under way"), "{err}");
    assert!(super::super::run::rollback(&fx.home, None).unwrap_err().contains("under way"));
    drop(held);
    assert!(super::apply(&fx.home).is_ok());
}
