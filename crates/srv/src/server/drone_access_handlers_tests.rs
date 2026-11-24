// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::server::drone_handlers::{recorded_as, upsert_drone_as};

fn drone(state: &AppState, by: &RpcContext) -> String {
    let def = serde_json::from_value(serde_json::json!({ "id": "", "name": "d", "created_at": 0, "updated_at": 0 })).unwrap();
    upsert_drone_as(state, def, &recorded_as(by)).unwrap().id
}

fn agent(id: &str) -> RpcContext {
    RpcContext { agent_id: id.into(), ..Default::default() }
}

fn give(state: &AppState, ctx: &RpcContext, id: &str, agents: &[&str]) -> Result<DroneAccess, String> {
    set_drone_agents(state, ctx, SetDroneAgentsReq { drone_id: id.into(), agents: agents.iter().map(|a| a.to_string()).collect() })
}

#[tokio::test]
async fn the_person_gives_a_drone_to_agents_and_the_list_is_replaced_whole() {
    let state = crate::server::tests::test_state();
    let window = RpcContext::operator_for_tests();
    let id = drone(&state, &window);
    let got = give(&state, &window, &id, &["Korp", "clamk", " "]).unwrap();
    assert_eq!(got.owner, "");
    assert_eq!(got.agents, ["clamk", "korp"]);
    assert_eq!(give(&state, &window, &id, &["opaz"]).unwrap().agents, ["opaz"]);
    assert_eq!(get_drone_access(&state, GetDroneAccessReq { drone_id: id }).unwrap().agents, ["opaz"]);
}

#[tokio::test]
async fn the_owner_gives_its_drone_away_and_another_agent_cannot() {
    let state = crate::server::tests::test_state();
    let id = drone(&state, &agent("opaz"));
    let given = give(&state, &agent("OPAZ"), &id, &["korp"]).unwrap();
    assert_eq!(given.owner, "opaz");
    assert_eq!(given.agents, ["korp"]);
    for other in ["korp", "clamk"] {
        let err = give(&state, &agent(other), &id, &["clamk"]).unwrap_err();
        assert!(err.starts_with("FORBIDDEN"), "{other}: {err}");
    }
    let err = give(&state, &RpcContext::default(), &id, &["clamk"]).unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "a connection that proved nothing: {err}");
    assert_eq!(get_drone_access(&state, GetDroneAccessReq { drone_id: id }).unwrap().agents, ["korp"], "nothing changed");
}

#[tokio::test]
async fn a_drone_nobody_made_cannot_be_given_by_an_agent_and_an_unknown_drone_is_not_found() {
    let state = crate::server::tests::test_state();
    let window = RpcContext::operator_for_tests();
    let id = drone(&state, &window);
    assert!(give(&state, &agent("opaz"), &id, &["opaz"]).unwrap_err().starts_with("FORBIDDEN"), "no owner, so no agent gives it");
    assert!(give(&state, &window, "ghost", &["opaz"]).unwrap_err().starts_with("NOT_FOUND"));
    let too_many: Vec<String> = (0..=MAX_AGENTS).map(|i| format!("a{i}")).collect();
    let err = set_drone_agents(&state, &window, SetDroneAgentsReq { drone_id: id, agents: too_many }).unwrap_err();
    assert!(err.starts_with("INVALID_ARG"), "{err}");
}
