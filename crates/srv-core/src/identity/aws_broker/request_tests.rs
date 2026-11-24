// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::identity::aws_broker::policy::Hold;
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
    Ask { agent_uid: "uid-jcont", agent_slug: "jcont", in_container: true, profile, reason, tag: None, seconds: None }
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
    assert_eq!(r.message, format!("AWS credentials took too long ({} s). Try again in a minute.", mint::RECIPE_TIMEOUT.as_secs()));

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

#[tokio::test]
async fn a_grant_that_is_held_or_refused_is_never_minted() {
    for (hold, code) in [(Hold::Awaiting, "GRANT_HELD"), (Hold::Refused, "GRANT_REFUSED")] {
        let mut p = Policy::parse(POLICY).unwrap();
        p.grants.get_mut("deploy").unwrap().hold = Some(hold);
        let fake = Fake::ok(creds_json(30));
        let r = credentials_for(Ok(Some(Arc::new(p))), &Sessions::default(), &fake, &ask("deploy", Some("why")), now()).await.err().expect("refused");
        assert_eq!((r.status, r.code), (StatusCode::FORBIDDEN, code));
        assert!(r.message.contains("Identity aws_grants"), "{}", r.message);
        assert_eq!(fake.runs(), 0, "nothing is minted for a held grant");
    }
}

/// A grant that takes one tag from the caller. The key and prefix are invented.
const TAGGED: &str = r#"{ "version": 1, "grants": {
    "scoped": { "tier": "read", "mint": { "kind": "command", "argv": ["/opt/mint", "--tag", "{tag_key}={tag}", "--reason", "{reason}"] }, "agents": ["jcont"], "needs_reason": true,
                "caller_tag": { "key": "Scope", "prefix": "team-a/" } },
    "plain": { "tier": "read", "mint": { "kind": "command", "argv": ["/opt/read"] }, "agents": ["*"] }
} }"#;

async fn tagged(fake: &Fake, sessions: &Sessions, tag: Option<&str>, reason: Option<&str>) -> Result<Granted, Refusal> {
    let policy = Ok(Some(Arc::new(Policy::parse(TAGGED).unwrap())));
    credentials_for(policy, sessions, fake, &Ask { tag, ..ask("scoped", reason) }, now()).await
}

#[tokio::test]
async fn a_grant_that_takes_a_tag_mints_with_the_callers_value_and_records_it() {
    let fake = Fake::ok(creds_json(30));
    let g = tagged(&fake, &Sessions::default(), Some(" team-a/db/password "), Some("check the rotation")).await.unwrap();
    assert_eq!(g.tag, Some(("Scope".to_string(), "team-a/db/password".to_string())));
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen[0].1, ["--tag", "Scope=team-a/db/password", "--reason", "check the rotation"]);
}

#[tokio::test]
async fn a_value_that_is_missing_has_other_characters_or_another_prefix_is_refused_with_a_sentence_and_not_minted() {
    let long = format!("team-a/{}", "x".repeat(250));
    for (value, says) in [
        (None, "needs the Scope tag"),
        (Some("  "), "needs the Scope tag"),
        (Some("team-a/has space"), "only letters, digits and _.:/=+-@"),
        (Some("team-a/*"), "only letters, digits and _.:/=+-@"),
        (Some("team-a/x,Other=y"), "only letters, digits and _.:/=+-@"),
        (Some("team-a/new\nline"), "only letters, digits and _.:/=+-@"),
        (Some(long.as_str()), "at most 256 characters"),
        (Some("team-b/other"), "has to start with 'team-a/'"),
        (Some("TEAM-A/upper"), "has to start with 'team-a/'"),
    ] {
        let fake = Fake::ok(creds_json(30));
        let r = tagged(&fake, &Sessions::default(), value, Some("why")).await.err().unwrap_or_else(|| panic!("{value:?} was granted"));
        assert_eq!((r.status, r.code), (StatusCode::BAD_REQUEST, "TAG_REFUSED"), "{value:?}");
        assert!(r.message.contains(says), "{value:?}: {}", r.message);
        assert_eq!(fake.runs(), 0, "{value:?}: nothing was minted");
    }
    // 256 characters is allowed.
    let edge = format!("team-a/{}", "x".repeat(249));
    assert!(tagged(&Fake::ok(creds_json(30)), &Sessions::default(), Some(&edge), Some("why")).await.is_ok());
}

#[tokio::test]
async fn a_session_is_for_one_value_and_a_grant_that_takes_no_tag_ignores_one() {
    let fake = Fake::ok(creds_json(30));
    let sessions = Sessions::default();
    let first = tagged(&fake, &sessions, Some("team-a/one"), Some("why")).await.unwrap();
    let again = tagged(&fake, &sessions, Some("team-a/one"), Some("why")).await.unwrap();
    let other = tagged(&fake, &sessions, Some("team-a/two"), Some("why")).await.unwrap();
    assert_eq!((first.source, again.source, other.source), (Source::Fresh, Source::Reused, Source::Fresh));
    assert_eq!(fake.runs(), 2, "another value is another mint");
    // A grant with no tag mints as it always did, whatever the caller sent.
    let policy = Ok(Some(Arc::new(Policy::parse(TAGGED).unwrap())));
    let plain = credentials_for(policy, &Sessions::default(), &Fake::ok(creds_json(30)), &Ask { tag: Some("team-a/x"), ..ask("plain", None) }, now()).await.unwrap();
    assert_eq!(plain.tag, None);
}
