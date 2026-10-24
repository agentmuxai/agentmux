// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::backend::storage::store::AgentContent;
use crate::registry::DefinitionStore;

fn input(role: &str) -> SeatInput {
    SeatInput { role: role.to_string(), ..Default::default() }
}

/// `leads` is each agent's current lead; every key and value is an agent.
fn check(agent: &str, seat: &SeatInput, leads: &[(&str, &str)]) -> Result<AgentSeat, String> {
    let leads: HashMap<String, String> = leads.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let known = |id: &str| ["a", "b", "c", "d"].contains(&id);
    validate(agent, seat, 42, known, |id| leads.get(id).cloned())
}

#[test]
fn a_valid_seat_is_normalized_and_stamped() {
    let seat = SeatInput {
        role: " leader ".into(),
        qualifier: Some("  product ".into()),
        lead: Some("  ".into()),
        area: Some(" agent pane,\n identity ".into()),
        lane: Some(String::new()),
    };
    let got = check("a", &seat, &[]).unwrap();
    assert_eq!(
        got,
        AgentSeat {
            role: "leader".into(),
            qualifier: Some("product".into()),
            lead: None,
            area: Some("agent pane, identity".into()),
            lane: None,
            set_at_ms: 42,
        }
    );
}

#[test]
fn every_role_is_accepted_and_an_unknown_one_refused() {
    for role in ROLES {
        assert!(check("a", &input(role), &[]).is_ok(), "{role}");
    }
    let err = check("a", &input("boss"), &[]).unwrap_err();
    assert!(err.contains("unknown role 'boss'"), "{err}");
}

#[test]
fn a_lead_must_be_another_agent_on_this_machine() {
    let mut seat = input("worker");
    seat.lead = Some("a".into());
    assert!(check("a", &seat, &[]).unwrap_err().contains("its own lead"));
    seat.lead = Some("ghost".into());
    assert!(check("a", &seat, &[]).unwrap_err().contains("not an agent on this machine"));
    seat.lead = Some("b".into());
    assert_eq!(check("a", &seat, &[]).unwrap().lead.as_deref(), Some("b"));
}

#[test]
fn a_lead_that_reports_to_the_agent_is_a_cycle() {
    let mut seat = input("worker");
    seat.lead = Some("c".into());
    // c reports to b, b to a: a can't report to c.
    let err = check("a", &seat, &[("c", "b"), ("b", "a")]).unwrap_err();
    assert!(err.contains("cycle"), "{err}");
    // A chain that reaches the operator is fine.
    assert!(check("a", &seat, &[("c", "b")]).is_ok());
    // A cycle above, not through this agent, isn't this seat's to refuse.
    assert!(check("a", &seat, &[("c", "d"), ("d", "c")]).is_ok());
}

#[test]
fn qualifier_area_and_lane_have_limits() {
    let mut seat = input("specialist");
    seat.qualifier = Some("q".repeat(MAX_QUALIFIER_CHARS));
    seat.area = Some("é".repeat(MAX_AREA_CHARS));
    seat.lane = Some("l".repeat(MAX_LANE_CHARS));
    assert!(check("a", &seat, &[]).is_ok(), "the limits themselves are allowed (in characters)");
    for (field, s) in [
        ("qualifier", SeatInput { qualifier: Some("q".repeat(MAX_QUALIFIER_CHARS + 1)), ..input("analyst") }),
        ("area", SeatInput { area: Some("a".repeat(MAX_AREA_CHARS + 1)), ..input("analyst") }),
        ("lane", SeatInput { lane: Some("l".repeat(MAX_LANE_CHARS + 1)), ..input("analyst") }),
    ] {
        let err = check("a", &s, &[]).unwrap_err();
        assert!(err.contains(field), "{err}");
    }
    // Surrounding blanks don't count toward a limit.
    let padded = SeatInput { qualifier: Some(format!("  {}  ", "q".repeat(MAX_QUALIFIER_CHARS))), ..input("analyst") };
    assert!(check("a", &padded, &[]).is_ok());
}

fn store_with_registry() -> (Store, tempfile::TempDir) {
    let store = Store::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    store.set_def_registry(Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap()));
    (store, tmp)
}

fn agent(store: &Store, id: &str, name: &str) -> AgentDefinition {
    let mut def = crate::backend::storage::agents::test_agent_def(id, name, "claude", "host", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    def
}

fn make_worker_of(store: &Store, worker: &str, leader: &str) {
    store
        .agent_content_set(&AgentContent {
            agent_id: worker.into(),
            content_type: crate::server::app_api::agent_spawn::WORKER_OF_CONTENT.into(),
            content: serde_json::json!({ "agent": "Leader", "id": leader }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

fn listed(store: &Store, id: &str) -> AgentDefinition {
    store.agent_def_list().unwrap().into_iter().find(|d| d.id == id).unwrap()
}

#[test]
fn a_spawned_worker_without_a_seat_reports_to_its_leader() {
    let (store, _tmp) = store_with_registry();
    agent(&store, "lead-1", "Leader");
    agent(&store, "work-1", "Worker");
    make_worker_of(&store, "work-1", "lead-1");

    let seat = effective_seat(&store, &listed(&store, "work-1")).unwrap();
    assert_eq!(seat.role, "worker");
    assert_eq!(seat.qualifier.as_deref(), Some(SPAWNED_QUALIFIER));
    assert_eq!(seat.lead.as_deref(), Some("lead-1"));
    assert_eq!(seat.set_at_ms, 0, "an effective seat is never stored");
    assert!(store.agent_seat_get("work-1").is_none(), "computed, not stored");
    assert_eq!(effective_seat_by_id(&store, "work-1"), Some(seat));
    // The leader itself has none.
    assert!(effective_seat(&store, &listed(&store, "lead-1")).is_none());
}

#[test]
fn a_stored_seat_wins_over_the_spawned_one() {
    let (store, _tmp) = store_with_registry();
    agent(&store, "lead-1", "Leader");
    agent(&store, "work-1", "Worker");
    make_worker_of(&store, "work-1", "lead-1");
    let stored = AgentSeat { role: "specialist".into(), area: Some("docs".into()), set_at_ms: 9, ..Default::default() };
    store.agent_seat_set("work-1", Some(&stored)).unwrap();

    assert_eq!(effective_seat(&store, &listed(&store, "work-1")), Some(stored.clone()));
    assert_eq!(effective_seat_by_id(&store, "work-1"), Some(stored));
}

#[test]
fn summary_in_names_the_lead() {
    let (store, _tmp) = store_with_registry();
    agent(&store, "lead-1", "Camper");
    agent(&store, "work-1", "Worker");
    let seat = AgentSeat { role: "worker".into(), lead: Some("lead-1".into()), area: Some("panes".into()), ..Default::default() };
    store.agent_seat_set("work-1", Some(&seat)).unwrap();
    let defs = store.agent_def_list().unwrap();
    let got = summary_in(&store, &defs, "work-1").unwrap();
    assert_eq!(
        got,
        SeatSummary {
            role: "worker".into(),
            qualifier: None,
            lead: Some(SeatLead { id: "lead-1".into(), name: Some("Camper".into()) }),
            area: Some("panes".into()),
        }
    );
    assert!(summary_in(&store, &defs, "lead-1").is_none());
    // A lead this machine doesn't know keeps its uid, with no name.
    let lone = summarize(&AgentSeat { role: "worker".into(), lead: Some("far".into()), ..Default::default() }, |_| None);
    assert_eq!(lone.lead, Some(SeatLead { id: "far".into(), name: None }));
}

#[test]
fn a_peer_summary_is_bounded_and_needs_a_known_role() {
    let ok = serde_json::json!({
        "role": "leader",
        "qualifier": "q".repeat(100),
        "lead": { "id": "uid-1", "name": "Camper\nX" },
        "area": "a".repeat(500),
    });
    let got = sanitize_peer_summary(&ok, 64).unwrap();
    assert_eq!(got.qualifier.as_ref().map(|q| q.chars().count()), Some(MAX_QUALIFIER_CHARS));
    assert_eq!(got.area.as_ref().map(|a| a.chars().count()), Some(MAX_AREA_CHARS));
    assert_eq!(got.lead, Some(SeatLead { id: "uid-1".into(), name: Some("Camper X".into()) }));

    for bad in [
        serde_json::json!({ "role": "boss", "qualifier": null, "lead": null, "area": null }),
        serde_json::json!({ "role": 7 }),
        serde_json::json!("leader"),
    ] {
        assert!(sanitize_peer_summary(&bad, 64).is_none(), "{bad}");
    }
    // A lead with a blank id is dropped, the seat kept.
    let blank = serde_json::json!({ "role": "worker", "qualifier": null, "lead": { "id": " ", "name": null }, "area": null });
    assert_eq!(sanitize_peer_summary(&blank, 64).unwrap().lead, None);
}

fn pane(oid: &str, view: &str, agent_id: &str, seat: Option<serde_json::Value>) -> crate::backend::obj::Block {
    let mut block = crate::backend::obj::Block { oid: oid.into(), ..Default::default() };
    block.meta.insert("view".into(), serde_json::json!(view));
    block.meta.insert("agentId".into(), serde_json::json!(agent_id));
    if let Some(seat) = seat {
        block.meta.insert(SEAT_META_KEY.into(), seat);
    }
    block
}

#[test]
fn only_agent_panes_whose_seat_changed_are_patched() {
    let leader = SeatSummary { role: "leader".into(), qualifier: Some("product".into()), lead: None, area: None };
    let current = seat_meta(Some(&leader));
    let blocks = vec![
        // Up to date: left alone.
        pane("b1", "agent", "a", Some(current.clone())),
        // Same agent, no seat yet: given it.
        pane("b2", "agent", "a", None),
        // Its seat was cleared: the key is removed (null).
        pane("b3", "agent", "b", Some(current.clone())),
        // No seat and none wanted: left alone.
        pane("b4", "agent", "c", None),
        // Not an agent pane, or an empty one: never touched.
        pane("b5", "term", "a", None),
        pane("b6", "agent", "", None),
    ];
    let mut asked = Vec::new();
    let stale = stale_seat_meta(&blocks, |id| {
        asked.push(id.to_string());
        (id == "a").then(|| leader.clone())
    });
    assert_eq!(stale, vec![("b2".to_string(), current), ("b3".to_string(), serde_json::Value::Null)]);
    assert_eq!(asked, vec!["a", "b", "c"], "each agent is looked up once");
}

#[test]
fn a_pane_is_seeded_with_its_agents_seat() {
    let (store, _tmp) = store_with_registry();
    agent(&store, "lead-1", "Camper");
    agent(&store, "work-1", "Worker");
    make_worker_of(&store, "work-1", "lead-1");
    let mut meta = crate::backend::obj::MetaMapType::new();
    seed_meta(&store, "work-1", &mut meta);
    let seeded: SeatSummary = serde_json::from_value(meta[SEAT_META_KEY].clone()).unwrap();
    assert_eq!(seeded.qualifier.as_deref(), Some(SPAWNED_QUALIFIER));
    assert_eq!(seeded.lead, Some(SeatLead { id: "lead-1".into(), name: Some("Camper".into()) }));
    let mut none = crate::backend::obj::MetaMapType::new();
    seed_meta(&store, "lead-1", &mut none);
    assert!(none.is_empty(), "no seat, no key");
}
