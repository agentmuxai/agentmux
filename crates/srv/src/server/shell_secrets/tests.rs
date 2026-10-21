// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A command given Vault secrets: asked once per entry, signed, local, and the
//! values scrubbed from its output.

use std::collections::HashMap;

use super::*;
use crate::backend::vault::seal;
use crate::backend::vault::{create, NewEntry};
use agentmux_common::api_types::UiAutomationAuth;

const VALUE: &str = "sk_live_abcdefgh1234";

struct Setup {
    state: AppState,
    block: String,
    auth: UiAutomationAuth,
    entry: String,
}

async fn setup() -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let agent = format!("runner-{}", uuid::Uuid::new_v4());
    let block = format!("block-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    let auth = UiAutomationAuth { agent_id: agent, ts_secs: ts, sig };
    let entry = create(
        &state.identity_store,
        NewEntry {
            kind: "api-key".into(),
            label: format!("Stripe {}", uuid::Uuid::new_v4().simple()),
            fields: Default::default(),
            tags: vec![],
            parts: seal::Parts([("value".to_string(), VALUE.to_string())].into()),
        },
    )
    .unwrap();
    Setup { state, block, auth, entry }
}

fn wanted(s: &Setup) -> HashMap<String, String> {
    HashMap::from([("STRIPE_API_KEY".to_string(), s.entry.clone())])
}

async fn run(s: &Setup, wanted: &HashMap<String, String>, remote: bool) -> Result<Option<Injected>, Refusal> {
    inject(&s.state, &s.block, Some(&s.auth), Some(wanted), remote, "stripe balance retrieve", "C:/work").await
}

/// Answers `agent`'s waiting use as the person would, and says what it showed.
async fn answer_when_asked(agent: String, decision: Decision) -> UseRequest {
    for _ in 0..200 {
        if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
            uses::uses().answer(&u.id, uses::UseAnswer::of(decision));
            return u;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the use was never asked about");
}

fn activity(s: &Setup) -> String {
    s.state.identity_store.vault_activity_list(50).unwrap().iter().map(|a| format!("{} {} {} {}", a.actor, a.action, a.target, a.outcome)).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn a_first_use_asks_with_the_command_and_then_gives_the_value_to_that_process_only() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let injected = run(&s, &wanted(&s), false).await.ok().flatten().expect("allowed");
    let shown = ask.await.unwrap();
    assert_eq!((shown.command.as_str(), shown.cwd.as_str(), shown.field.as_str()), ("stripe balance retrieve", "C:/work", "STRIPE_API_KEY (value)"));
    assert!(shown.origin.is_empty() && shown.offer_always && !shown.needs_value);
    assert_eq!(injected.env, vec![("STRIPE_API_KEY".to_string(), VALUE.to_string())]);
    // The output is cleaned of it, and the log says a command used it.
    assert_eq!(injected.scrubber.scrub(&format!("using {VALUE}")).as_ref(), format!("using [vault:{}]", s.entry));
    let log = activity(&s);
    assert!(log.contains("ran a command stripe balance retrieve done") && !log.contains(VALUE), "{log}");
    // Once is once: the next command asks again.
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    assert!(run(&s, &wanted(&s), false).await.is_ok());
    ask.await.unwrap();
}

#[tokio::test]
async fn always_is_a_grant_for_this_agent_and_the_entry_and_asks_nothing_again() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Always));
    assert!(run(&s, &wanted(&s), false).await.is_ok());
    ask.await.unwrap();
    assert!(s.state.identity_store.vault_grant_check(&s.entry, &grant_key(&s.auth.agent_id, "value")).unwrap());
    assert!(!s.state.identity_store.vault_grant_check(&s.entry, &grant_key("someone-else", "value")).unwrap(), "a grant is one agent's");
    // No one answers now, and it still runs.
    let again = tokio::time::timeout(Duration::from_secs(5), run(&s, &wanted(&s), false)).await.expect("didn't wait for an answer");
    assert!(again.ok().flatten().is_some());
}

#[tokio::test]
async fn no_from_the_person_means_the_command_does_not_run() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    let err = run(&s, &wanted(&s), false).await.err().expect("refused");
    ask.await.unwrap();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert!(err.1.contains("did not run") && !err.1.contains(VALUE));
    assert!(activity(&s).contains("denied"));
}

#[tokio::test]
async fn a_command_is_refused_secrets_it_cannot_safely_have() {
    let s = setup().await;
    // Another machine.
    let err = run(&s, &wanted(&s), true).await.err().unwrap();
    assert!(err.1.contains("WSL or an SSH host"), "{err:?}");
    // No signed identity, or another agent's.
    let unsigned = inject(&s.state, &s.block, None, Some(&wanted(&s)), false, "x", "").await.err().unwrap();
    assert_eq!(unsigned.0, StatusCode::FORBIDDEN);
    let other = setup().await;
    let borrowed = inject(&s.state, &s.block, Some(&other.auth), Some(&wanted(&s)), false, "x", "").await.err().unwrap();
    assert_eq!(borrowed.0, StatusCode::FORBIDDEN);
    // A name that isn't a variable, one that is AgentMux's, an entry that isn't there, a part it lacks.
    for (name, entry, why) in [
        ("1BAD", s.entry.as_str(), "usable environment variable"),
        ("AGENTMUX_AUTH_KEY", s.entry.as_str(), "usable environment variable"),
        ("KEY", "no-such-entry", "no Vault entry"),
        ("KEY", &format!("{}.nonsense", s.entry), "has no part"),
    ] {
        let err = run(&s, &HashMap::from([(name.to_string(), entry.to_string())]), false).await.err().unwrap_or_else(|| panic!("{name} accepted"));
        assert_eq!(err.0, StatusCode::BAD_REQUEST, "{name}: {err:?}");
        assert!(err.1.contains(why), "{name}: {err:?}");
    }
    // Nothing was asked of the person for any of those.
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
}

#[tokio::test]
async fn a_value_too_short_to_scrub_is_not_handed_over() {
    let s = setup().await;
    let short = create(
        &s.state.identity_store,
        NewEntry { kind: "api-key".into(), label: "Tiny".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "abc".to_string())].into()) },
    )
    .unwrap();
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let err = run(&s, &HashMap::from([("K".to_string(), short)]), false).await.err().expect("refused");
    ask.await.unwrap();
    assert!(err.1.contains("too short"), "{err:?}");
}

#[tokio::test]
async fn nothing_given_is_nothing_injected() {
    let s = setup().await;
    assert!(inject(&s.state, &s.block, None, None, false, "x", "").await.ok().flatten().is_none());
    assert!(inject(&s.state, &s.block, None, Some(&HashMap::new()), false, "x", "").await.ok().flatten().is_none());
}

// A card's expiry is not its number: an Always for one part covers only that part, and
// one question names every part the command asks for.
#[tokio::test]
async fn a_grant_is_for_one_part_and_the_question_names_every_part_asked_for() {
    let s = setup().await;
    let card = create(
        &s.state.identity_store,
        NewEntry {
            kind: "card".into(),
            label: format!("Visa {}", uuid::Uuid::new_v4().simple()),
            fields: Default::default(),
            tags: vec![],
            parts: seal::Parts([("number".to_string(), "4111111111111111".to_string()), ("expiry".to_string(), "12/2030".to_string()), ("cvc".to_string(), "123456".to_string())].into()),
        },
    )
    .unwrap();
    let expiry_only = HashMap::from([("CARD_EXPIRY".to_string(), format!("{card}.expiry"))]);
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Always));
    assert!(run(&s, &expiry_only, false).await.is_ok());
    ask.await.unwrap();
    assert!(s.state.identity_store.vault_grant_check(&card, &grant_key(&s.auth.agent_id, "expiry")).unwrap());
    assert!(!s.state.identity_store.vault_grant_check(&card, &grant_key(&s.auth.agent_id, "number")).unwrap());

    // Asking for the number as well asks again, naming what isn't yet allowed: the number, not the expiry.
    let both = HashMap::from([("CARD_EXPIRY".to_string(), format!("{card}.expiry")), ("CARD_NUMBER".to_string(), format!("{card}.number"))]);
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    assert!(run(&s, &both, false).await.is_err());
    let shown = ask.await.unwrap();
    assert!(shown.field.contains("CARD_NUMBER (number)") && !shown.field.contains("CARD_EXPIRY"), "{}", shown.field);
}

// The prompt shows the whole command or the command does not run: a line past a cut-off
// could send the value anywhere, unseen.
#[tokio::test]
async fn a_command_too_long_to_show_whole_is_refused_without_asking() {
    let s = setup().await;
    let long = format!("echo hi {}", "x".repeat(MAX_COMMAND));
    let err = inject(&s.state, &s.block, Some(&s.auth), Some(&wanted(&s)), false, &long, "C:/work").await.err().expect("refused");
    assert!(err.1.contains("fit in the approval prompt"), "{err:?}");
    let far = "d".repeat(MAX_CWD + 1);
    let err = inject(&s.state, &s.block, Some(&s.auth), Some(&wanted(&s)), false, "echo hi", &far).await.err().expect("refused");
    assert!(err.1.contains("fit in the approval prompt"), "{err:?}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
    // At the limit it is shown whole.
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let at_limit = "y".repeat(MAX_COMMAND);
    assert!(inject(&s.state, &s.block, Some(&s.auth), Some(&wanted(&s)), false, &at_limit, "C:/work").await.is_ok());
    assert_eq!(ask.await.unwrap().command, at_limit);
}
