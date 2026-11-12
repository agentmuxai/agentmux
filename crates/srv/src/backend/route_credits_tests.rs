// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A model route out of credits (#47449).

use super::*;
use crate::agents::failure::{classify, FailureClass, ROUTE_CREDITS_TITLE};
use crate::backend::storage::agents::test_agent_def;
use crate::server::AppState;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

/// What Claude Code printed on the DeepSeek route when the key's credits ran out.
const NO_CREDITS: &str = "API Error: 402 This request would exceed your available credits. This request requires more credits, or fewer max_tokens.";

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

async fn tab(state: &AppState) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

async fn pane(state: &AppState, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn agent(state: &AppState, id: &str, name: &str) {
    let mut def = test_agent_def(id, name, "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
}

fn serve(state: &AppState, worker_id: &str, lead_id: &str) {
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker_id.to_string(),
            content_type: crate::server::app_api::agent_spawn::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": "lead", "id": lead_id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

#[test]
fn the_402_texts_routes_print_are_the_credits_failure_and_a_plan_limit_is_not() {
    let deke = classify(Some(1), None, NO_CREDITS, None);
    assert_eq!((deke.code, deke.title.as_str()), (FailureClass::UsageLimit, ROUTE_CREDITS_TITLE), "{deke:?}");
    assert!(deke.is_route_credits());
    for text in [
        "API Error: 402 Insufficient Balance",
        "HTTP 402 Payment Required",
        "Error: 402 {\"error\":{\"message\":\"Insufficient credits. Add more at https://openrouter.ai/settings/credits\"}}",
        "This request would exceed your available credits",
    ] {
        assert!(classify(Some(1), None, text, None).is_route_credits(), "{text}");
    }
    let plan = classify(Some(1), None, "You've hit your usage limit. Upgrade to Pro or visit the settings to purchase more credits or try again at 9:44 PM.", None);
    assert!(!plan.is_route_credits(), "a plan limit that mentions credits keeps its own reading");
    assert!(!classify(Some(1), None, "API Error: 429 rate limited", None).is_route_credits());
}

#[test]
fn only_a_credits_failure_event_with_its_pane_is_picked_up() {
    let f = classify(Some(1), None, NO_CREDITS, None);
    let data = serde_json::to_value(&f).unwrap();
    let event = crate::backend::mps::EVENT_AGENT_FAILURE;
    let scopes = vec!["block:b1".to_string()];
    assert_eq!(credits_failure(event, &scopes, Some(&data)).map(|(b, _)| b), Some("b1".to_string()));
    assert!(credits_failure(event, &[], Some(&data)).is_none(), "no pane");
    assert!(credits_failure("agentturn", &scopes, Some(&data)).is_none());
    let other = serde_json::to_value(classify(Some(1), None, "You've hit your weekly limit", None)).unwrap();
    assert!(credits_failure(event, &scopes, Some(&other)).is_none(), "another limit");
}

/// The first 402 marks the other agents on the route and nobody else; a second opens nothing; the first success
/// takes the failure off again and the next 402 is news again.
#[tokio::test]
async fn the_first_402_marks_every_agent_on_the_route_and_the_first_success_clears_them() {
    let state = crate::server::tests::test_state();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (a, b, c) = (format!("a-{id}"), format!("b-{id}"), format!("c-{id}"));
    for (def, name) in [(&a, "Alpha"), (&b, "Bravo"), (&c, "Charlie")] {
        agent(&state, def, name);
    }
    launch::set_choice(&state.mstore, &a, "claude", "openrouter", "").unwrap();
    launch::set_choice(&state.mstore, &b, "claude", "openrouter", "").unwrap();
    let (pa, pb, pc) = (pane(&state, &tab, &a).await, pane(&state, &tab, &b).await, pane(&state, &tab, &c).await);
    let failure = classify(Some(1), None, NO_CREDITS, None);
    let now = agentmux_common::time::now_ms_u64();

    let opened = on_failure(&state.mstore, &state.event_bus, &pa, &failure, now).expect("the first 402 opens the episode");
    assert_eq!(opened.route.as_deref(), Some(catalog::route("openrouter").unwrap().label));
    let mut names: Vec<_> = opened.agents.iter().map(|(_, n)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Alpha", "Bravo"], "the agents on the route, not the one on none");
    assert!(shows_credits(&state.mstore, &pb), "the other agent on the route now shows it");
    assert!(!shows_credits(&state.mstore, &pc), "an agent on no route does not");
    let held_until = failure_of_block(&state.mstore.get::<Block>(&pb).unwrap().unwrap()).and_then(|f| f.resets_at_ms);
    assert!(held_until.is_some_and(|at| at > now + 9 * 60 * 1000), "its messages wait for a reset: {held_until:?}");

    assert_eq!(on_failure(&state.mstore, &state.event_bus, &pb, &failure, now + 1).map(|_| ()), None, "a second 402 is the same episode");

    cleared(&state.mstore, &state.event_bus, "openrouter");
    assert!(!shows_credits(&state.mstore, &pb), "the first success takes it off");
    cleared(&state.mstore, &state.event_bus, "openrouter");
    assert!(on_failure(&state.mstore, &state.event_bus, &pa, &failure, now + 2).is_some(), "the next 402 is news again");
    cleared(&state.mstore, &state.event_bus, "openrouter");
}

/// A model switch recorded during a turn names the destination as the agent's choice while its harness still runs
/// the route it left: a late 402 belongs to the route left, not to the healthy destination (#47472).
#[tokio::test]
async fn a_402_after_a_switch_during_a_turn_is_the_route_left_not_the_destination() {
    let state = crate::server::tests::test_state();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (a, b, c) = (format!("a-{id}"), format!("b-{id}"), format!("c-{id}"));
    for (def, name) in [(&a, "Alpha"), (&b, "Bravo"), (&c, "Charlie")] {
        agent(&state, def, name);
    }
    // Routes of its own, since the episodes are process-wide and another test opens openrouter's.
    let left = launch::Choice { route: "left-route".into(), model: String::new() };
    // set_choice only takes catalog routes, so the choices are written as stored.
    let destination = launch::Choice { route: "destination-route".into(), model: String::new() };
    let put = |def: &str, content_type: &str, choice: &launch::Choice| {
        state
            .mstore
            .agent_content_set(&crate::backend::storage::store::AgentContent {
                agent_id: def.to_string(),
                content_type: content_type.to_string(),
                content: serde_json::to_string(choice).unwrap(),
                updated_at: 1,
            })
            .unwrap();
    };
    put(&a, launch::CONTENT_TYPE, &destination);
    put(&a, launch::LEAVING_TYPE, &left);
    put(&b, launch::CONTENT_TYPE, &left);
    put(&c, launch::CONTENT_TYPE, &destination);
    let (pa, pb, pc) = (pane(&state, &tab, &a).await, pane(&state, &tab, &b).await, pane(&state, &tab, &c).await);
    let failure = classify(Some(1), None, NO_CREDITS, None);

    let opened = on_failure(&state.mstore, &state.event_bus, &pa, &failure, agentmux_common::time::now_ms_u64()).expect("opens the route's episode");
    assert_eq!(opened.route.as_deref(), Some("left-route"), "the route it was running");
    assert!(shows_credits(&state.mstore, &pb), "the agent still on the failing route is marked");
    assert!(!shows_credits(&state.mstore, &pc), "the healthy destination is not");
    cleared(&state.mstore, &state.event_bus, "left-route");
}

/// An agent on its own key never passes the gateway, so a turn that works is what ends its episode: the next credits
/// failure inside the episode's six hours is news again (#47473).
#[tokio::test]
async fn a_standalone_agents_next_failure_is_news_again_after_a_turn_that_worked() {
    let state = crate::server::tests::test_state();
    let tab = tab(&state).await;
    let def = format!("solo-{}", uuid::Uuid::new_v4());
    agent(&state, &def, "Solo");
    let block = pane(&state, &tab, &def).await;
    let failure = classify(Some(1), None, NO_CREDITS, None);
    let now = agentmux_common::time::now_ms_u64();

    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now).is_some(), "the first failure opens the episode");
    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 1).is_none(), "a second is the same episode");
    turn_succeeded(&state.mstore, &block);
    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 2).is_some(), "after a turn that worked it is news again");
    turn_succeeded(&state.mstore, &block);
}

#[tokio::test]
async fn an_agent_on_no_route_has_an_episode_of_its_own_and_one_that_ran_long_enough_is_let_go() {
    let state = crate::server::tests::test_state();
    let tab = tab(&state).await;
    let def = format!("own-{}", uuid::Uuid::new_v4());
    agent(&state, &def, "Dee");
    let p = pane(&state, &tab, &def).await;
    let failure = classify(Some(1), None, "API Error: 402 Insufficient Balance", None);
    let opened = on_failure(&state.mstore, &state.event_bus, &p, &failure, 1_000).expect("opens");
    assert_eq!((opened.route, opened.agents), (None, vec![(def.clone(), "Dee".to_string())]));
    assert!(on_failure(&state.mstore, &state.event_bus, &p, &failure, 2_000).is_none());
    assert!(on_failure(&state.mstore, &state.event_bus, &p, &failure, 1_000 + EPISODE_MAX_MS + 1).is_some(), "an episode no success ended is let go");
}

#[tokio::test]
async fn each_lead_is_told_once_about_its_own_agents_and_an_agent_with_no_lead_tells_no_one() {
    let state = crate::server::tests::test_state();
    let id = uuid::Uuid::new_v4().to_string();
    let (lead, other_lead, a, b, c, loner) = (
        format!("lead-{id}"),
        format!("other-{id}"),
        format!("a-{id}"),
        format!("b-{id}"),
        format!("c-{id}"),
        format!("loner-{id}"),
    );
    for (def, name) in [(&lead, "Lead"), (&other_lead, "Other"), (&a, "Alpha"), (&b, "Bravo"), (&c, "Charlie"), (&loner, "Loner")] {
        agent(&state, def, name);
    }
    serve(&state, &a, &lead);
    serve(&state, &b, &lead);
    serve(&state, &c, &other_lead);
    let agents = [(&a, "Alpha"), (&b, "Bravo"), (&c, "Charlie"), (&loner, "Loner")].map(|(d, n)| (d.clone(), n.to_string()));
    let opened = Opened { route: Some("OpenRouter".into()), agents: agents.to_vec() };
    let notes = notes_for(&state.mstore, &opened);
    assert_eq!(notes.len(), 2, "one per lead; the agent with none is skipped: {notes:?}");
    let first = notes.iter().find(|(d, _)| *d == a).expect("the first lead's note goes through its first agent");
    assert!(first.1.contains("OpenRouter is out of credits (HTTP 402), so Alpha, Bravo can't work (4 on the route in all)"), "{}", first.1);
    assert!(!first.1.contains("Charlie"), "only its own agents");
    assert!(notes.iter().any(|(d, body)| *d == c && body.contains("Charlie can't work")));
    let alone = notes_for(&state.mstore, &Opened { route: None, agents: vec![(a.clone(), "Alpha".into())] });
    assert!(alone[0].1.starts_with("Alpha is out of credits on its model provider's account"), "{}", alone[0].1);
}
