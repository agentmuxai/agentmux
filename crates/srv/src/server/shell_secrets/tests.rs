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

/// Values for the environment only, as `secrets` alone names them.
fn env(m: &HashMap<String, String>) -> Wanted<'_> {
    Wanted { env: Some(m), ..Default::default() }
}

async fn run(s: &Setup, wanted: &HashMap<String, String>, remote: bool) -> Result<Option<Injected>, Refusal> {
    inject(&s.state, &s.block, Some(&s.auth), env(wanted), remote, "stripe balance retrieve", "C:/work", None).await
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
    let unsigned = inject(&s.state, &s.block, None, env(&wanted(&s)), false, "x", "", None).await.err().unwrap();
    assert_eq!(unsigned.0, StatusCode::FORBIDDEN);
    let other = setup().await;
    let borrowed = inject(&s.state, &s.block, Some(&other.auth), env(&wanted(&s)), false, "x", "", None).await.err().unwrap();
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
    assert!(inject(&s.state, &s.block, None, Wanted::default(), false, "x", "", None).await.ok().flatten().is_none());
    let none = HashMap::new();
    assert!(inject(&s.state, &s.block, None, env(&none), false, "x", "", None).await.ok().flatten().is_none());
    assert!(inject(&s.state, &s.block, None, Wanted { stdin: Some("  "), ..Default::default() }, false, "x", "", None).await.ok().flatten().is_none());
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
    let err = inject(&s.state, &s.block, Some(&s.auth), env(&wanted(&s)), false, &long, "C:/work", None).await.err().expect("refused");
    assert!(err.1.contains("fit in the approval prompt"), "{err:?}");
    let far = "d".repeat(MAX_CWD + 1);
    let err = inject(&s.state, &s.block, Some(&s.auth), env(&wanted(&s)), false, "echo hi", &far, None).await.err().expect("refused");
    assert!(err.1.contains("fit in the approval prompt"), "{err:?}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
    // At the limit it is shown whole.
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let at_limit = "y".repeat(MAX_COMMAND);
    assert!(inject(&s.state, &s.block, Some(&s.auth), env(&wanted(&s)), false, &at_limit, "C:/work", None).await.is_ok());
    assert_eq!(ask.await.unwrap().command, at_limit);
}

// A job given secrets keeps its cleaned output for the agent whose pane started it, and no other job does.
#[tokio::test]
async fn the_cleaned_output_is_read_back_only_by_the_agent_that_ran_the_job() {
    use crate::backend::shell_node::ShellStatusInfo;
    use agentmux_common::api_types::ShellOutputRequest;
    use axum::response::IntoResponse;
    let s = setup().await;
    let tail = std::sync::Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::from(["token=[vault:x]".to_string(), "done".to_string()])));
    let status = std::sync::Arc::new(parking_lot::Mutex::new(ShellStatusInfo { running: false, exit_code: Some(0), line_count: 2, block_id: s.block.clone(), tail: Some(tail), tail_clipped: true, ..Default::default() }));
    let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
    s.state.shell_sessions.register_full("job-with-secrets".to_string(), tx, None, status);
    let plain = std::sync::Arc::new(parking_lot::Mutex::new(ShellStatusInfo { running: false, exit_code: Some(0), line_count: 1, block_id: s.block.clone(), ..Default::default() }));
    let (tx2, _rx2) = tokio::sync::oneshot::channel::<()>();
    s.state.shell_sessions.register_full("job-without".to_string(), tx2, None, plain);

    let ask = |id: &str, auth: &UiAutomationAuth| {
        let req = ShellOutputRequest { shell_id: id.to_string(), auth: auth.clone() };
        let state = s.state.clone();
        async move {
            let resp = crate::server::http_shell::handle_shell_output(axum::extract::State(state), axum::Json(req)).await.into_response();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default())
        }
    };
    let (code, body) = ask("job-with-secrets", &s.auth).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!((body["output"].as_str(), body["exit_code"].as_i64(), body["running"].as_bool(), body["truncated"].as_bool()), (Some("token=[vault:x]\ndone"), Some(0), Some(false), Some(true)));
    // Another pane, a job that kept nothing, and an id nobody has.
    // Another agent, really known to this srv, with its own signed identity.
    let other_agent = format!("other-{}", uuid::Uuid::new_v4());
    s.state.reactive_handler.register_agent(&other_agent, &format!("block-{}", uuid::Uuid::new_v4()), None).unwrap();
    let key = s.state.mstore.agent_jekt_key_ensure(&other_agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &other_agent, "__srv__", ts, "");
    let other = UiAutomationAuth { agent_id: other_agent, ts_secs: ts, sig };
    assert_eq!(ask("job-with-secrets", &other).await.0, StatusCode::NOT_FOUND, "another agent's own identity");
    assert_eq!(ask("job-without", &s.auth).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ask("no-such-job", &s.auth).await.0, StatusCode::NOT_FOUND);
    // The instance key is not an identity: a request that only names an agent is refused.
    let forged = UiAutomationAuth { agent_id: s.auth.agent_id.clone(), ts_secs: s.auth.ts_secs, sig: "not-a-signature".to_string() };
    assert_eq!(ask("job-with-secrets", &forged).await.0, StatusCode::FORBIDDEN);
}

// SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.3: stdin and files are
// asked about, read and scrubbed the same way as the environment.
#[tokio::test]
async fn stdin_and_files_are_given_their_values_after_the_same_question() {
    let s = setup().await;
    let files = HashMap::from([("KEY_FILE".to_string(), s.entry.clone())]);
    let want = Wanted { env: None, stdin: Some(s.entry.as_str()), files: Some(&files) };
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let injected = inject(&s.state, &s.block, Some(&s.auth), want, false, "docker login --password-stdin", "C:/work", None).await.ok().flatten().expect("allowed");
    let shown = ask.await.unwrap();
    assert_eq!(shown.field, "stdin (value), a file at KEY_FILE (value)");
    assert!(injected.env.is_empty());
    assert_eq!(injected.stdin.as_deref().map(String::as_str), Some(VALUE));
    assert_eq!(injected.files.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect::<Vec<_>>(), vec![("KEY_FILE", VALUE)]);
    assert_eq!(injected.scrubber.scrub(VALUE).as_ref(), format!("[vault:{}]", s.entry));
}

#[tokio::test]
async fn a_name_given_both_as_a_variable_and_as_a_file_is_refused_without_asking() {
    let s = setup().await;
    let both = wanted(&s);
    let want = Wanted { env: Some(&both), stdin: None, files: Some(&both) };
    let err = inject(&s.state, &s.block, Some(&s.auth), want, false, "x", "", None).await.err().expect("refused");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("both secrets and secret_files"), "{err:?}");
    let bad = HashMap::from([("not a name".to_string(), s.entry.clone())]);
    let err = inject(&s.state, &s.block, Some(&s.auth), Wanted { files: Some(&bad), ..Default::default() }, false, "x", "", None).await.err().expect("refused");
    assert!(err.1.contains("usable environment variable"), "{err:?}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
}

fn capture_rule(into: agentmux_common::api_types::CaptureInto) -> Vec<agentmux_common::api_types::CaptureRule> {
    vec![agentmux_common::api_types::CaptureRule { into, from: agentmux_common::api_types::CaptureFrom::Stdout("all".into()) }]
}

#[tokio::test]
async fn a_capture_into_a_new_entry_asks_nobody_but_needs_the_agents_signature() {
    let s = setup().await;
    let rules = capture_rule(agentmux_common::api_types::CaptureInto { kind: Some("api-key".into()), label: Some("GitHub token".into()), ..Default::default() });
    let job = tokio::time::timeout(Duration::from_secs(5), plan_capture(&s.state, &s.block, Some(&s.auth), Some(&rules), "gh auth token", "C:/work"))
        .await
        .expect("didn't wait for an answer")
        .expect("planned")
        .expect("a job");
    assert_eq!(job.capture.agent, s.auth.agent_id);
    let unsigned = plan_capture(&s.state, &s.block, None, Some(&rules), "gh auth token", "").await.err().expect("refused");
    assert_eq!(unsigned.0, StatusCode::FORBIDDEN);
    assert!(plan_capture(&s.state, &s.block, None, Some(&[]), "true", "").await.unwrap().is_none(), "no rules, no capture");
    let bad = capture_rule(agentmux_common::api_types::CaptureInto { kind: Some("api-key".into()), ..Default::default() });
    assert_eq!(plan_capture(&s.state, &s.block, Some(&s.auth), Some(&bad), "true", "").await.err().expect("refused").0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_capture_that_replaces_a_value_asks_each_time_naming_the_command() {
    let s = setup().await;
    let rules = capture_rule(agentmux_common::api_types::CaptureInto { id: Some(s.entry.clone()), ..Default::default() });
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    assert!(plan_capture(&s.state, &s.block, Some(&s.auth), Some(&rules), "stripe keys roll", "C:/work").await.unwrap().is_some());
    let shown = ask.await.unwrap();
    assert!(shown.replaces && !shown.offer_always, "replacing a value is asked about every time");
    assert_eq!((shown.entry_id.as_str(), shown.command.as_str(), shown.cwd.as_str()), (s.entry.as_str(), "stripe keys roll", "C:/work"));
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    let err = plan_capture(&s.state, &s.block, Some(&s.auth), Some(&rules), "stripe keys roll", "C:/work").await.err().expect("refused");
    ask.await.unwrap();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert!(activity(&s).contains("captured stripe keys roll denied"), "{}", activity(&s));
}

#[tokio::test]
async fn a_rotations_approval_covers_exactly_its_own_secrets() {
    let s = setup().await;
    crate::backend::vault::versions::start_rotation(&s.state.identity_store, &s.entry, "sk_live_pending_value_0001", &s.auth.agent_id).unwrap();
    let pending = format!("{}@pending", s.entry);
    let own = HashMap::from([("NEW_SECRET".to_string(), pending.clone()), ("OLD_SECRET".to_string(), s.entry.clone())]);
    let approved = Preapproved { env: own.clone() };
    // Its own command runs without another question, with the pending value.
    let ran = tokio::time::timeout(Duration::from_secs(5), inject(&s.state, &s.block, Some(&s.auth), env(&own), false, "set-it", "", Some(&approved)))
        .await
        .expect("nothing was asked");
    assert!(ran.is_ok_and(|i| i.is_some()));
    // One more secret than it approved isn't covered, so the pending value isn't given.
    let more = HashMap::from([("NEW_SECRET".to_string(), pending.clone()), ("OTHER".to_string(), s.entry.clone())]);
    let err = inject(&s.state, &s.block, Some(&s.auth), env(&more), false, "set-it", "", Some(&approved)).await.err().expect("refused");
    assert!(err.1.contains("@pending"), "{}", err.1);
    // And without the rotation's approval, nobody gets it.
    let alone = HashMap::from([("NEW_SECRET".to_string(), pending)]);
    let err = inject(&s.state, &s.block, Some(&s.auth), env(&alone), false, "set-it", "", None).await.err().expect("refused");
    assert!(err.1.contains("@pending"), "{}", err.1);
}
