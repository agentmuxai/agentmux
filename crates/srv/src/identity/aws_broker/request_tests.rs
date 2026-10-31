// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::identity::aws_broker::mint::tests::{creds_json, now, Fake};
use crate::identity::aws_broker::mint::{RunError, RunOutput};

const POLICY: &str = r#"{ "version": 1, "default_grant": "read", "grants": {
    "read": { "tier": "read", "mint": { "kind": "command", "argv": ["/opt/read", "--seconds", "{seconds}"] }, "agents": ["*"] },
    "deploy": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/escalate", "--reason", "{reason}"] }, "agents": ["jcont"], "max_mints_per_hour": 2 },
    "prod": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/prod"] }, "agents": ["*"], "disabled": true, "note": "Production admin changes need the operator's decision." },
    "work": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["jcont"] }
} }"#;

fn policy() -> Result<Option<Arc<Policy>>, String> {
    Ok(Some(Arc::new(Policy::parse(POLICY).unwrap())))
}

fn ask<'a>(profile: &'a str, reason: Option<&'a str>) -> Ask<'a> {
    Ask { agent_uid: "uid-jcont", agent_slug: "jcont", in_container: true, profile, reason, seconds: None }
}

async fn go(fake: &Fake, sessions: &Sessions, ask: Ask<'_>) -> Result<Granted, Refusal> {
    credentials_for(policy(), sessions, fake, &ask, now()).await
}

async fn refusal(ask: Ask<'_>) -> Refusal {
    let fake = Fake::ok(creds_json(30));
    let r = go(&fake, &Sessions::default(), ask).await.err().expect("refused");
    assert_eq!(fake.runs(), 0, "nothing was minted for a refused request");
    r
}

#[tokio::test]
async fn a_native_agent_is_told_to_use_the_hosts_own_setup() {
    let r = refusal(Ask { in_container: false, ..ask("read", None) }).await;
    assert_eq!((r.status, r.code), (StatusCode::FORBIDDEN, "NATIVE_AGENT"));
    assert!(r.text().starts_with("NATIVE_AGENT: This is for container agents."), "{}", r.text());
}

#[tokio::test]
async fn without_a_policy_nothing_is_shared_and_a_broken_file_says_so() {
    let fake = Fake::ok(creds_json(30));
    for none in [Ok(None), Ok(Some(Arc::new(Policy::parse(r#"{ "version": 1, "grants": {} }"#).unwrap())))] {
        let r = credentials_for(none, &Sessions::default(), &fake, &ask("read", None), now()).await.err().unwrap();
        assert_eq!((r.status, r.code), (StatusCode::FORBIDDEN, "AWS_NOT_SHARED"));
        assert!(r.message.contains("~/.agentmux/aws-grants.json"));
    }
    let r = credentials_for(Err("aws-grants.json: line 3".into()), &Sessions::default(), &fake, &ask("read", None), now()).await.err().unwrap();
    assert_eq!((r.status, r.code), (StatusCode::SERVICE_UNAVAILABLE, "AWS_POLICY_UNREADABLE"));
    assert!(r.message.contains("line 3"));
    assert_eq!(fake.runs(), 0);
}

#[tokio::test]
async fn an_unknown_or_switched_off_grant_is_refused_with_what_is_there() {
    let r = refusal(ask("nope", None)).await;
    assert_eq!((r.status, r.code), (StatusCode::NOT_FOUND, "UNKNOWN_PROFILE"));
    assert_eq!(r.message, "No AWS grant named 'nope'. This machine shares: deploy, prod, read, work.");
    let r = refusal(ask("prod", Some("why"))).await;
    assert_eq!((r.status, r.code), (StatusCode::FORBIDDEN, "GRANT_DISABLED"));
    assert!(r.message.ends_with("Production admin changes need the operator's decision."), "{}", r.message);
}

#[tokio::test]
async fn a_grant_the_agent_is_not_named_in_is_refused_naming_who_has_it() {
    let r = refusal(Ask { agent_slug: "lazo", agent_uid: "uid-lazo", ..ask("deploy", Some("why")) }).await;
    assert_eq!((r.status, r.code), (StatusCode::FORBIDDEN, "NOT_ALLOWED_FOR_YOU"));
    assert!(r.message.contains("It is open to: jcont."), "{}", r.message);
}

#[tokio::test]
async fn a_write_grant_without_a_reason_says_how_to_give_one() {
    for reason in [None, Some(""), Some("  ;;  ")] {
        let r = refusal(ask("deploy", reason)).await;
        assert_eq!((r.status, r.code), (StatusCode::BAD_REQUEST, "REASON_REQUIRED"));
        assert!(r.message.contains("AGENTMUX_AWS_REASON='what and why'"), "{}", r.message);
    }
}

#[tokio::test]
async fn a_read_grant_needs_no_reason_and_the_lifetime_is_held_to_the_grants_bounds() {
    let fake = Fake::ok(creds_json(60));
    let s = Sessions::default();
    let g = go(&fake, &s, ask("read", None)).await.unwrap();
    assert_eq!((g.seconds, g.source, g.reason.as_str()), (3600, Source::Fresh, ""));
    assert_eq!(fake.seen.lock().unwrap()[0].1, ["--seconds", "3600"]);
    // Asked for more or less than the bounds: held to them.
    let fake = Fake::ok(creds_json(60));
    let g = go(&fake, &Sessions::default(), Ask { seconds: Some(99_999), ..ask("read", None) }).await.unwrap();
    assert_eq!(g.seconds, 3600);
    let g = go(&fake, &Sessions::default(), Ask { seconds: Some(10), ..ask("read", None) }).await.unwrap();
    assert_eq!(g.seconds, 900);
}

#[tokio::test]
async fn a_second_ask_with_the_same_reason_is_the_same_session_and_a_new_reason_is_a_new_one() {
    let fake = Fake::ok(creds_json(30));
    let s = Sessions::default();
    let first = go(&fake, &s, ask("deploy", Some("deploy stack X!"))).await.unwrap();
    let second = go(&fake, &s, ask("deploy", Some("deploy stack X"))).await.unwrap();
    assert_eq!((first.source, second.source), (Source::Fresh, Source::Reused), "the cleaned reasons are the same");
    assert_eq!(fake.runs(), 1);
    assert_eq!(fake.seen.lock().unwrap()[0].1, ["--reason", "deploy stack X"], "what the recipe was given is the cleaned reason");
    let other = go(&fake, &s, ask("deploy", Some("deploy stack Y"))).await.unwrap();
    assert_eq!(other.source, Source::Fresh);
    assert_eq!(fake.runs(), 2);
}

#[tokio::test]
async fn the_hourly_cap_is_a_refusal_with_the_number() {
    let fake = Fake::ok(creds_json(30));
    let s = Sessions::default();
    for n in 0..2 {
        go(&fake, &s, ask("deploy", Some(&format!("reason {n}")))).await.unwrap();
    }
    let r = go(&fake, &s, ask("deploy", Some("reason 3"))).await.err().unwrap();
    assert_eq!((r.status, r.code), (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_MINTS"));
    assert!(r.message.contains("2 times in the last hour"), "{}", r.message);
}

#[tokio::test]
async fn what_goes_wrong_on_the_host_reads_as_the_hosts_problem() {
    let s = Sessions::default();
    let r = go(&Fake::failing("AccessDenied: not allowed"), &s, ask("deploy", Some("x"))).await.err().unwrap();
    assert_eq!((r.status, r.code), (StatusCode::SERVICE_UNAVAILABLE, "AWS_HOST_REFUSED"));
    assert_eq!(r.message, "The host's AWS refused: AccessDenied: not allowed");

    let r = go(&Fake::failing("Error loading SSO Token: expired"), &Sessions::default(), ask("work", None)).await.err().unwrap();
    assert_eq!(r.code, "AWS_HOST_LOGIN_NEEDED");
    assert!(r.message.contains("aws sso login --profile work"), "{}", r.message);

    let mut missing = Fake::ok(String::new());
    missing.which = None;
    let r = go(&missing, &Sessions::default(), ask("work", None)).await.err().unwrap();
    assert_eq!(r.code, "AWS_CLI_MISSING");
    assert!(r.message.contains("v2.9"), "{}", r.message);

    let r = go(&Fake::answering(Err(RunError::TimedOut)), &Sessions::default(), ask("deploy", Some("x"))).await.err().unwrap();
    assert_eq!(r.code, "AWS_TOOK_TOO_LONG");
    assert_eq!(r.message, "AWS credentials took too long (25 s). Try again in a minute.");

    let r = go(
        &Fake::answering(Ok(RunOutput { success: true, stdout: b"hello".to_vec(), stderr: String::new() })),
        &Sessions::default(),
        ask("deploy", Some("x")),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(r.code, "AWS_HOST_REFUSED");
}

#[tokio::test]
async fn no_refusal_or_answer_text_carries_a_credential() {
    let fake = Fake::failing("AccessDenied");
    let r = go(&fake, &Sessions::default(), ask("deploy", Some("x"))).await.err().unwrap();
    let granted = go(&Fake::ok(creds_json(30)), &Sessions::default(), ask("read", None)).await.unwrap();
    for text in [r.text(), format!("{:?}", granted.creds)] {
        assert!(!text.contains("AKIATESTKEY") && !text.contains("test-secret-value") && !text.contains("test-session-token"), "{text}");
    }
}
