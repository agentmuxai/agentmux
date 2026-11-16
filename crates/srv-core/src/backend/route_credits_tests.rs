// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A model route out of credits (#47449).

use super::*;
use crate::agents::failure::{classify, FailureClass, ROUTE_CREDITS_TITLE};
use crate::backend::storage::agents::test_agent_def;
use crate::test_support::CoreParts;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

/// What Claude Code printed on the DeepSeek route when the key's credits ran out.
const NO_CREDITS: &str = "API Error: 402 This request would exceed your available credits. This request requires more credits, or fewer max_tokens.";

async fn dispatch_apply(state: &CoreParts, cmd: Command) -> Vec<Event> {
    state.dispatch_apply(cmd).await
}

async fn tab(state: &CoreParts) -> String {
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

async fn pane(state: &CoreParts, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn agent(state: &CoreParts, id: &str, name: &str) {
    let mut def = test_agent_def(id, name, "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
}

fn serve(state: &CoreParts, worker_id: &str, lead_id: &str) {
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker_id.to_string(),
            content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": "lead", "id": lead_id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

/// What the launch of the pane's harness records on it: the route it was started on.
fn launched_on(state: &CoreParts, block_id: &str, route: &str) {
    let mut update = crate::backend::obj::MetaMapType::new();
    update.insert(launch::BLOCK_META_KEY.to_string(), json!({ "route": route }));
    crate::backend::wcore::object_meta::update_object_meta(&state.mstore, &format!("block:{block_id}"), &update).unwrap();
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
    let state = crate::test_support::core_parts();
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
    launched_on(&state, &pa, "openrouter");
    launched_on(&state, &pb, "openrouter");

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

    cleared(&state.mstore, &state.event_bus, &state.broker, "openrouter");
    assert!(!shows_credits(&state.mstore, &pb), "the first success takes it off");
    cleared(&state.mstore, &state.event_bus, &state.broker, "openrouter");
    assert!(on_failure(&state.mstore, &state.event_bus, &pa, &failure, now + 2).is_some(), "the next 402 is news again");
    cleared(&state.mstore, &state.event_bus, &state.broker, "openrouter");
}

/// A model switch is recorded for the agent at once, but each of its panes is relaunched on it when its own turn
/// ends. A 402 on the route left marks the panes still on it, wherever the agent's saved choice points (#47474).
#[tokio::test]
async fn a_402_after_a_switch_marks_the_panes_still_on_the_route_left_and_not_the_ones_relaunched() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (a, b, c) = (format!("a-{id}"), format!("b-{id}"), format!("c-{id}"));
    for (def, name) in [(&a, "Alpha"), (&b, "Bravo"), (&c, "Charlie")] {
        agent(&state, def, name);
    }
    // Routes of their own, since the episodes are process-wide and another test opens openrouter's.
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
    // Alpha has switched, and still holds the mark of the route it left because one of its panes is mid-turn.
    put(&a, launch::CONTENT_TYPE, &destination);
    put(&a, launch::LEAVING_TYPE, &left);
    put(&b, launch::CONTENT_TYPE, &left);
    put(&c, launch::CONTENT_TYPE, &destination);
    let (busy, relaunched) = (pane(&state, &tab, &a).await, pane(&state, &tab, &a).await);
    let (pb, pc) = (pane(&state, &tab, &b).await, pane(&state, &tab, &c).await);
    launched_on(&state, &busy, "left-route");
    launched_on(&state, &relaunched, "destination-route");
    launched_on(&state, &pb, "left-route");
    launched_on(&state, &pc, "destination-route");
    let failure = classify(Some(1), None, NO_CREDITS, None);

    let opened = on_failure(&state.mstore, &state.event_bus, &busy, &failure, agentmux_common::time::now_ms_u64()).expect("opens the route's episode");
    assert_eq!(opened.route.as_deref(), Some("left-route"), "the route its harness was launched on");
    assert!(shows_credits(&state.mstore, &pb), "the agent still on the failing route is marked");
    assert!(!shows_credits(&state.mstore, &relaunched), "the pane of the same agent already relaunched on the new route is not");
    assert!(!shows_credits(&state.mstore, &pc), "the healthy destination is not");
    cleared(&state.mstore, &state.event_bus, &state.broker, "left-route");
}

/// An agent on its own key never passes the gateway, so a turn that works is what ends its episode: the next credits
/// failure inside the episode's six hours is news again (#47473).
#[tokio::test]
async fn a_standalone_agents_next_failure_is_news_again_after_a_turn_that_worked() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let def = format!("solo-{}", uuid::Uuid::new_v4());
    agent(&state, &def, "Solo");
    let block = pane(&state, &tab, &def).await;
    let failure = classify(Some(1), None, NO_CREDITS, None);
    let now = agentmux_common::time::now_ms_u64();

    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now).is_some(), "the first failure opens the episode");
    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 1).is_none(), "a second is the same episode");
    turn_succeeded(&state.mstore, Some(&state.broker), &block);
    assert!(on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 2).is_some(), "after a turn that worked it is news again");
    turn_succeeded(&state.mstore, Some(&state.broker), &block);
}

#[tokio::test]
async fn an_agent_on_no_route_has_an_episode_of_its_own_and_one_that_ran_long_enough_is_let_go() {
    let state = crate::test_support::core_parts();
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
    let state = crate::test_support::core_parts();
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
    let opened = Opened { key: "openrouter".into(), route: Some("OpenRouter".into()), agents: agents.to_vec() };
    let notes = notes_for(&state.mstore, &opened);
    assert_eq!(notes.len(), 2, "one per lead; the agent with none is skipped: {notes:?}");
    let first = notes.iter().find(|(d, _)| *d == a).expect("the first lead's note goes through its first agent");
    assert!(first.1.contains("OpenRouter is out of credits (HTTP 402), so Alpha, Bravo can't work (4 on the route in all)"), "{}", first.1);
    assert!(!first.1.contains("Charlie"), "only its own agents");
    assert!(notes.iter().any(|(d, body)| *d == c && body.contains("Charlie can't work")));
    let alone = notes_for(&state.mstore, &Opened { key: format!("agent:{a}"), route: None, agents: vec![(a.clone(), "Alpha".into())] });
    assert!(alone[0].1.starts_with("Alpha is out of credits on its model provider's account"), "{}", alone[0].1);
}

/// The person is told once, in words that name the route and the fix but no agent and no key (#47476).
#[test]
fn the_person_is_told_the_route_and_what_to_do_and_no_agent_by_name() {
    let on_route = Opened { key: "openrouter".into(), route: Some("OpenRouter".into()), agents: vec![("a".into(), "Alpha".into()), ("b".into(), "Bravo".into())] };
    let note = person_note(&on_route);
    assert!(note.starts_with("OpenRouter is out of credits, so 2 agents can't work. Add credits"), "{note}");
    assert!(!note.contains("Alpha") && !note.contains("Bravo"), "{note}");
    let alone = person_note(&Opened { key: "agent:a".into(), route: None, agents: vec![("a".into(), "Alpha".into())] });
    assert!(alone.starts_with("An agent is out of credits on its model provider's account"), "{alone}");
    assert!(!alone.contains("Alpha"), "{alone}");
    let one = person_note(&Opened { key: "x".into(), route: Some("OpenRouter".into()), agents: vec![("a".into(), "Alpha".into())] });
    assert!(one.contains("so 1 agent can't work"), "{one}");
    assert_ne!(note_key("openrouter"), note_key("agent:a"));
    assert!(!note_key("openrouter").contains("openrouter"), "hashed, like an account's key");
}

/// A success can land after the episode opens and before the person's note is posted (the leads are told in
/// between). The note it could not take down is then not posted at all, and an episode that is still open posts it
/// (#47508).
#[tokio::test]
async fn a_success_before_the_note_is_posted_leaves_no_note_for_a_route_that_works() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let def = format!("a-{id}");
    agent(&state, &def, "Alpha");
    let block = pane(&state, &tab, &def).await;
    let failure = classify(Some(1), None, NO_CREDITS, None);
    let now = agentmux_common::time::now_ms_u64();
    let mut posts = 0;

    // An agent on no route: its own next turn works between the failure and the note.
    let opened = on_failure(&state.mstore, &state.event_bus, &block, &failure, now).expect("opens");
    turn_succeeded(&state.mstore, Some(&state.broker), &block);
    assert!(!post_while_open(&opened.key, || posts += 1));
    assert_eq!(posts, 0, "the episode ended first: nothing is posted");

    // A route: the gateway answers between the failure and the note.
    let route = format!("route-{id}");
    launched_on(&state, &block, &route);
    let opened = on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 1).expect("opens the route's episode");
    cleared(&state.mstore, &state.event_bus, &state.broker, &route);
    assert!(!post_while_open(&opened.key, || posts += 1));
    assert_eq!(posts, 0);

    // An episode still open posts, and the next success ends it.
    let opened = on_failure(&state.mstore, &state.event_bus, &block, &failure, now + 2).expect("news again");
    assert!(post_while_open(&opened.key, || posts += 1));
    assert_eq!(posts, 1);
    cleared(&state.mstore, &state.event_bus, &state.broker, &route);
}
