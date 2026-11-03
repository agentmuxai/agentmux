// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn command_register_roundtrip() {
    let c = Command::Register {
        kind: ClientKind::Host,
        pid: 12345,
        version: "0.33.449".into(),
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"cmd\":\"register\""));
    assert!(json.contains("\"kind\":\"host\""));
    let back: Command = serde_json::from_str(&json).unwrap();
    if let Command::Register { kind, pid, version } = back {
        assert_eq!(kind, ClientKind::Host);
        assert_eq!(pid, 12345);
        assert_eq!(version, "0.33.449");
    } else {
        panic!("wrong variant");
    }
}

#[test]
fn event_registered_roundtrip() {
    let e = Event::Registered {
        client_id: 1,
        launcher_pid: 9999,
        launcher_version: "0.33.449".into(),
        version: 42,
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"event\":\"registered\""));
    let back: Event = serde_json::from_str(&json).unwrap();
    if let Event::Registered { client_id, version, .. } = back {
        assert_eq!(client_id, 1);
        assert_eq!(version, 42);
    } else {
        panic!("wrong variant");
    }
}

#[test]
fn unknown_cmd_fails_to_deserialize() {
    let json = r#"{"cmd":"banana"}"#;
    let r: Result<Command, _> = serde_json::from_str(json);
    assert!(r.is_err());
}

// ---------- Phase CPD-1 — saga_id schema additions ----------

#[test]
fn cpd1_spawn_pool_window_round_trip_with_saga_id() {
    let c = Command::SpawnPoolWindow { saga_id: 42 };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"cmd\":\"spawn_pool_window\""));
    assert!(json.contains("\"saga_id\":42"));
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::SpawnPoolWindow { saga_id } => assert_eq!(saga_id, 42),
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_spawn_pool_window_forward_compat_default_zero() {
    // Pre-CPD-1 hosts emit the bare command with no `saga_id`
    // field; serde_default fills in 0.
    let json = r#"{"cmd":"spawn_pool_window"}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::SpawnPoolWindow { saga_id } => assert_eq!(saga_id, 0),
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_reap_panes_round_trip_with_saga_id() {
    let c = Command::ReapPanes {
        label: "main".into(),
        saga_id: 7,
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"cmd\":\"reap_panes\""));
    assert!(json.contains("\"saga_id\":7"));
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::ReapPanes { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, 7);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_reap_panes_forward_compat_default_zero() {
    let json = r#"{"cmd":"reap_panes","label":"main"}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::ReapPanes { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, 0);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_drain_pool_if_last_round_trip_with_saga_id() {
    let c = Command::DrainPoolIfLast {
        label: "main".into(),
        saga_id: 13,
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"cmd\":\"drain_pool_if_last\""));
    assert!(json.contains("\"saga_id\":13"));
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::DrainPoolIfLast { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, 13);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_drain_pool_if_last_forward_compat_default_zero() {
    let json = r#"{"cmd":"drain_pool_if_last","label":"main"}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::DrainPoolIfLast { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, 0);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_window_added_round_trip_with_some() {
    let c = Command::ReportPoolWindowAdded {
        label: "pool-xyz".into(),
        saga_id: Some(99),
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"saga_id\":99"));
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::ReportPoolWindowAdded { label, saga_id } => {
            assert_eq!(label, "pool-xyz");
            assert_eq!(saga_id, Some(99));
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_window_added_forward_compat_default_none() {
    // Pre-CPD-1 hosts omit `saga_id` entirely; deserializes to
    // `None`. JSON with explicit null is also valid.
    let json = r#"{"cmd":"report_pool_window_added","label":"pool-xyz"}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::ReportPoolWindowAdded { label, saga_id } => {
            assert_eq!(label, "pool-xyz");
            assert_eq!(saga_id, None);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_panes_reaped_round_trip_with_some() {
    let c = Command::ReportPanesReaped {
        label: "main".into(),
        saga_id: Some(11),
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::ReportPanesReaped { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, Some(11));
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_panes_reaped_forward_compat_default_none() {
    let json = r#"{"cmd":"report_panes_reaped","label":"main"}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::ReportPanesReaped { label, saga_id } => {
            assert_eq!(label, "main");
            assert_eq!(saga_id, None);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_drain_decision_round_trip_with_some() {
    let c = Command::ReportPoolDrainDecision {
        label: "main".into(),
        was_last: true,
        saga_id: Some(101),
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::ReportPoolDrainDecision {
            label,
            was_last,
            saga_id,
        } => {
            assert_eq!(label, "main");
            assert!(was_last);
            assert_eq!(saga_id, Some(101));
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_drain_decision_forward_compat_default_none() {
    let json = r#"{"cmd":"report_pool_drain_decision","label":"main","was_last":false}"#;
    let back: Command = serde_json::from_str(json).unwrap();
    match back {
        Command::ReportPoolDrainDecision {
            label,
            was_last,
            saga_id,
        } => {
            assert_eq!(label, "main");
            assert!(!was_last);
            assert_eq!(saga_id, None);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_report_saga_action_failed_round_trip() {
    let c = Command::ReportSagaActionFailed {
        saga_id: 55,
        reason: "window not found".into(),
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"cmd\":\"report_saga_action_failed\""));
    assert!(json.contains("\"saga_id\":55"));
    let back: Command = serde_json::from_str(&json).unwrap();
    match back {
        Command::ReportSagaActionFailed { saga_id, reason } => {
            assert_eq!(saga_id, 55);
            assert_eq!(reason, "window not found");
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_event_pool_window_added_round_trip_with_saga_id() {
    let e = Event::PoolWindowAdded {
        label: "pool-xyz".into(),
        version: 7,
        saga_id: Some(42),
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"saga_id\":42"));
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
}

#[test]
fn cpd1_event_panes_reaped_forward_compat_default_none() {
    // Pre-CPD-1 launcher-events.log entries lack `saga_id`.
    let json = r#"{"event":"panes_reaped","label":"main","version":1}"#;
    let back: Event = serde_json::from_str(json).unwrap();
    match back {
        Event::PanesReaped {
            label,
            version,
            saga_id,
        } => {
            assert_eq!(label, "main");
            assert_eq!(version, 1);
            assert_eq!(saga_id, None);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_event_pool_drained_round_trip_with_saga_id() {
    let e = Event::PoolDrained {
        label: "main".into(),
        version: 9,
        saga_id: Some(3),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
}

#[test]
fn cpd1_event_pool_not_last_forward_compat_default_none() {
    let json = r#"{"event":"pool_not_last","label":"main","version":3}"#;
    let back: Event = serde_json::from_str(json).unwrap();
    match back {
        Event::PoolNotLast {
            label,
            version,
            saga_id,
        } => {
            assert_eq!(label, "main");
            assert_eq!(version, 3);
            assert_eq!(saga_id, None);
        }
        other => panic!("wrong variant: {:?}", other),
    }
}

#[test]
fn cpd1_event_saga_action_failed_round_trip() {
    let e = Event::SagaActionFailed {
        saga_id: 12,
        reason: "host pipe broken".into(),
        version: 100,
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"event\":\"saga_action_failed\""));
    assert!(json.contains("\"saga_id\":12"));
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
}

#[test]
fn cpd1_host_frame_event_round_trip() {
    let frame = HostFrame::Event(Event::PoolWindowAdded {
        label: "pool-xyz".into(),
        version: 5,
        saga_id: Some(7),
    });
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"kind\":\"event\""));
    assert!(json.contains("\"event\":\"pool_window_added\""));
    let back: HostFrame = serde_json::from_str(&json).unwrap();
    match back {
        HostFrame::Event(Event::PoolWindowAdded {
            label,
            version,
            saga_id,
        }) => {
            assert_eq!(label, "pool-xyz");
            assert_eq!(version, 5);
            assert_eq!(saga_id, Some(7));
        }
        other => panic!("wrong frame: {:?}", other),
    }
}

#[test]
fn cpd1_host_frame_command_round_trip() {
    let frame = HostFrame::Command(Command::SpawnPoolWindow { saga_id: 31 });
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"kind\":\"command\""));
    assert!(json.contains("\"cmd\":\"spawn_pool_window\""));
    assert!(json.contains("\"saga_id\":31"));
    let back: HostFrame = serde_json::from_str(&json).unwrap();
    match back {
        HostFrame::Command(Command::SpawnPoolWindow { saga_id }) => {
            assert_eq!(saga_id, 31);
        }
        other => panic!("wrong frame: {:?}", other),
    }
}

#[test]
fn cpd1_host_frame_unknown_kind_fails() {
    let json = r#"{"kind":"banana"}"#;
    let r: Result<HostFrame, _> = serde_json::from_str(json);
    assert!(r.is_err());
}
