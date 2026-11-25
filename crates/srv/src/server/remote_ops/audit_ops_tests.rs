// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

fn peer(host: &str, channel: &str, install: &str) -> LanInstance {
    // Each is its own instance: an address and port serve one instance.
    static NEXT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
    let address = format!("192.0.2.{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut p: LanInstance = serde_json::from_value(json!({
        "instance_id": "v0", "hostname": host, "version": "0.60.7", "channel": channel, "os": "windows",
        "address": address, "port": 29700, "auth_key": "k", "agents": [], "first_seen": 0, "last_seen": 0,
        "other_ttl_secs": 4500
    }))
    .unwrap();
    p.install = install.to_string();
    p
}

#[test]
fn each_computer_is_asked_once_and_named_so_the_send_finds_exactly_one_instance() {
    let instances = [
        peer("alpha", "stable", ""),
        peer("beta", "stable", "windows-store"),
        peer("beta", "dev", ""),
        peer("gamma", "stable", "windows-store"),
        peer("gamma", "stable", "windows-setup"),
        peer("  ", "stable", ""),
    ];
    let hosts = lan_hosts(&instances);
    assert_eq!(
        hosts,
        [
            ("alpha".to_string(), "alpha".to_string()),
            ("beta".to_string(), "beta/stable".to_string()),
            ("gamma".to_string(), "gamma/stable/windows-store".to_string()),
        ]
    );
    // What `send` takes finds one instance for each.
    for (_, host) in &hosts {
        assert!(crate::server::remote_ops::send::resolve_peer(&instances, host).is_ok(), "{host}");
    }
}

#[test]
fn a_computers_answer_is_its_rows_or_the_reason_there_are_none() {
    let page = json!({ "entries": [{ "id": "j1", "ts_ms": 5, "kind": "jekt", "summary": "a message" }], "not_recorded": [], "channel": "stable" });
    let rows = rows_from("beta", RemoteOpResult::done(page));
    assert_eq!((rows.host.as_str(), rows.entries.len(), rows.note.as_deref()), ("beta", 1, None));
    assert_eq!(rows.entries[0].id, "beta/j1");

    let refused = rows_from("beta", RemoteOpResult::refused("not_shared"));
    assert!(refused.entries.is_empty() && refused.note.as_deref().unwrap().contains("sharing its Audit"), "{refused:?}");
    let failed = rows_from("beta", RemoteOpResult::failed("couldn't reach that machine"));
    assert_eq!(failed.note.as_deref(), Some("failed: couldn't reach that machine"));
    let garbled = rows_from("beta", RemoteOpResult::done(json!("not a page")));
    assert!(garbled.note.as_deref().unwrap().contains("not understood"), "{garbled:?}");
}
