// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use chrono::TimeZone;

use super::*;
use crate::identity::aws_broker::policy::Policy;

pub(crate) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
}

/// Credential JSON that expires `mins` minutes after [`now`].
pub(crate) fn creds_json(mins: i64) -> String {
    creds_json_from(now(), mins)
}

/// Credential JSON that expires `mins` minutes after `base` (the real clock, for a handler that reads it).
pub(crate) fn creds_json_from(base: DateTime<Utc>, mins: i64) -> String {
    let exp = (base + chrono::Duration::minutes(mins)).format("%Y-%m-%dT%H:%M:%SZ");
    format!(r#"{{"Version":1,"AccessKeyId":"AKIATESTKEY","SecretAccessKey":"test-secret-value","SessionToken":"test-session-token","Expiration":"{exp}"}}"#)
}

/// A runner that says what it is told to and remembers what it was asked.
pub(crate) struct Fake {
    pub which: Option<PathBuf>,
    pub answer: Mutex<Option<Result<RunOutput, RunError>>>,
    pub seen: Mutex<Vec<(PathBuf, Vec<String>, Vec<(String, String)>)>>,
}

impl Fake {
    pub fn ok(stdout: String) -> Self {
        Self::answering(Ok(RunOutput { success: true, stdout: stdout.into_bytes(), stderr: String::new() }))
    }

    pub fn answering(answer: Result<RunOutput, RunError>) -> Self {
        Self { which: Some(PathBuf::from("/bin/prog")), answer: Mutex::new(Some(answer)), seen: Mutex::new(Vec::new()) }
    }

    pub fn failing(stderr: &str) -> Self {
        Self::answering(Ok(RunOutput { success: false, stdout: Vec::new(), stderr: stderr.to_string() }))
    }

    pub fn runs(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl Runner for Fake {
    fn which(&self, _program: &str) -> Option<PathBuf> {
        self.which.clone()
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        self.seen.lock().unwrap().push((spec.program.clone(), spec.args.clone(), spec.env.clone()));
        let answer = self.answer.lock().unwrap();
        // A runner asked twice gives the same answer twice.
        match answer.as_ref().unwrap() {
            Ok(o) => Ok(RunOutput { success: o.success, stdout: o.stdout.clone(), stderr: o.stderr.clone() }),
            Err(RunError::TimedOut) => Err(RunError::TimedOut),
            Err(RunError::Spawn(e)) => Err(RunError::Spawn(e.clone())),
        }
    }
}

fn policy() -> Policy {
    Policy::parse(
        r#"{ "version": 1, "grants": {
            "read": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["*"] },
            "deploy": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/escalate", "--reason", "{reason}", "--duration", "{seconds}", "--session-name", "{session}", "--as", "{agent}"] }, "agents": ["jcont"] }
        } }"#,
    )
    .unwrap()
}

fn ctx<'a>(p: &'a Policy, grant: &str, reason: &'a str) -> MintCtx<'a> {
    MintCtx { agent_slug: "jcont", grant: &p.grants[grant], reason, seconds: 1800, now_ms: 1_793_448_000_000 }
}

#[test]
fn a_reason_is_cleaned_for_a_session_tag_and_never_starts_like_an_option() {
    assert_eq!(clean_reason("deploy stack X (staging)!"), "deploy stack X staging");
    assert_eq!(clean_reason("a;b | c\nd"), "a b c d");
    assert_eq!(clean_reason("--reason=x"), "reason=x", "the leading dashes go, so it can't read as an option");
    assert_eq!(clean_reason("  - fix "), "fix");
    assert_eq!(clean_reason(&"x".repeat(400)).chars().count(), 256);
    assert_eq!(clean_reason(""), "");
}

#[test]
fn placeholders_fill_whole_arguments_and_nothing_is_split_or_shell_read() {
    let p = policy();
    let c = ctx(&p, "deploy", "ship it; rm -rf / && echo $HOME `id` \"quoted\"");
    // The reason is cleaned before it gets here (the request path does it); the substitution itself must not split it.
    let reason = clean_reason(c.reason);
    let argv = recipe_argv(&MintCtx { reason: &reason, ..c });
    assert_eq!(argv[0], "/opt/escalate");
    assert_eq!(argv[1], "--reason");
    assert_eq!(argv[2], "ship it rm -rf / echo HOME id quoted", "{argv:?}");
    assert_eq!(argv.len(), 9, "the reason stayed one argument: {argv:?}");
    assert_eq!(argv[4], "1800");
    assert_eq!(argv[6], "jcont-deploy-1793448000000");
    assert_eq!(argv[8], "jcont");
}

#[test]
fn a_host_profile_runs_the_aws_clis_export_command_for_that_profile_only() {
    let p = policy();
    let argv = recipe_argv(&ctx(&p, "read", ""));
    assert_eq!(argv, ["aws", "configure", "export-credentials", "--profile", "work", "--format", "process"]);
}

#[test]
fn a_session_name_is_at_most_64_characters_and_keeps_its_time() {
    let mut p = policy();
    let long = "x".repeat(60);
    p.grants.get_mut("deploy").unwrap().name = long.clone();
    let c = MintCtx { agent_slug: "jcont", grant: &p.grants["deploy"], reason: "r", seconds: 900, now_ms: 1_793_448_000_000 };
    let name = c.session_name();
    assert!(name.len() <= 64, "{name}");
    assert!(name.ends_with("-1793448000000") && !name.starts_with('-'), "{name}");
}

#[test]
fn credential_json_is_read_with_a_banner_before_it_and_refused_when_it_does_not_expire() {
    let ok = parse_process_json(creds_json(60).as_bytes(), now()).unwrap();
    assert_eq!(ok.expiration, now() + chrono::Duration::minutes(60));
    assert_eq!(ok.to_json()["SessionToken"], "test-session-token");
    assert_eq!(ok.to_json()["Expiration"], "2026-10-31T13:00:00Z");
    assert!(parse_process_json(format!("some banner\n{}\n\n", creds_json(60)).as_bytes(), now()).is_ok());

    let refused = |text: &str| parse_process_json(text.as_bytes(), now()).unwrap_err();
    assert!(refused("").contains("nothing"));
    assert!(refused("not json").contains("not credential JSON"));
    assert!(refused(r#"{"Version":2}"#).contains("version 1"));
    assert!(refused(r#"{"Version":1,"AccessKeyId":"a","SecretAccessKey":"b","SessionToken":"c"}"#).contains("Expiration"), "a key that doesn't expire");
    assert!(refused(r#"{"Version":1,"AccessKeyId":"a","SecretAccessKey":"b","Expiration":"2026-10-31T13:00:00Z"}"#).contains("long-lived"), "no session token");
    assert!(refused(&creds_json(-5)).contains("already expired"));
    assert!(refused(r#"{"Version":1,"AccessKeyId":"a","SecretAccessKey":"b","SessionToken":"c","Expiration":"tomorrow"}"#).contains("not a time"));
}

#[test]
fn a_secret_is_not_in_the_debug_text() {
    let m = parse_process_json(creds_json(60).as_bytes(), now()).unwrap();
    let text = format!("{m:?}");
    assert!(!text.contains("AKIATESTKEY") && !text.contains("test-secret-value") && !text.contains("test-session-token"), "{text}");
}

#[tokio::test]
async fn a_recipe_is_run_with_its_arguments_and_the_agent_it_is_for() {
    let p = policy();
    let fake = Fake::ok(creds_json(30));
    let m = mint(&fake, &ctx(&p, "deploy", "deploy x"), now()).await.unwrap();
    assert_eq!(m.expiration, now() + chrono::Duration::minutes(30));
    let seen = fake.seen.lock().unwrap();
    let (program, args, env) = &seen[0];
    assert_eq!(program, &PathBuf::from("/bin/prog"));
    assert_eq!(args[..2], ["--reason".to_string(), "deploy x".to_string()]);
    assert_eq!(env, &[("AGENTMUX_AGENT_ID".to_string(), "jcont".to_string())]);
}

#[tokio::test]
async fn the_ways_a_mint_fails_are_told_apart() {
    let p = policy();
    let go = |fake: Fake, grant: &'static str| {
        let p = p.clone();
        async move { mint(&fake, &ctx(&p, grant, "r"), now()).await.unwrap_err() }
    };
    let mut none = Fake::ok(String::new());
    none.which = None;
    assert!(matches!(go(none, "read").await, MintError::CliMissing(m) if m.contains("AWS CLI")));
    let mut none = Fake::ok(String::new());
    none.which = None;
    assert!(matches!(go(none, "deploy").await, MintError::CliMissing(m) if m.contains("/opt/escalate")));

    assert!(matches!(go(Fake::failing("Error loading SSO Token: Token for work does not exist"), "read").await, MintError::LoginNeeded(_)));
    assert!(matches!(go(Fake::failing("aws: error: argument subcommand: Invalid choice, valid choices are: ..."), "read").await, MintError::CliMissing(m) if m.contains("too old")));
    assert!(matches!(go(Fake::failing("AccessDenied: not allowed"), "deploy").await, MintError::Refused(m) if m == "AccessDenied: not allowed"));
    assert!(matches!(go(Fake::failing(""), "deploy").await, MintError::Refused(m) if m.contains("said nothing")));
    assert!(matches!(go(Fake::ok("hello".into()), "deploy").await, MintError::Refused(m) if m.contains("not credential JSON")));
    assert!(matches!(go(Fake::answering(Err(RunError::TimedOut)), "deploy").await, MintError::TookTooLong));
    assert!(matches!(go(Fake::answering(Err(RunError::Spawn("denied".into()))), "deploy").await, MintError::Refused(m) if m.contains("denied")));

    let long = "e".repeat(900);
    match go(Fake::failing(&long), "deploy").await {
        MintError::Refused(m) => assert!(m.chars().count() <= 401, "the host's words are cut: {}", m.len()),
        other => panic!("{other:?}"),
    }
    // The login hint is only for the AWS CLI's own failures.
    assert!(matches!(go(Fake::failing("please sso login first"), "deploy").await, MintError::Refused(_)));
}

// The real runner, through this test binary: with `AWS_TEST_RECIPE_CHILD` set it is a program that prints credential
// JSON (or sleeps), and otherwise it does nothing.
#[test]
fn recipe_child() {
    match std::env::var("AWS_TEST_RECIPE_CHILD").as_deref() {
        Ok("creds") => {
            let leaked = std::env::var("AGENTMUX_AUTH_KEY").is_ok() || std::env::var("AGENTMUX_AGENT_TOKEN").is_ok();
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            // libtest has already printed "test <name> ... " without a newline: start our own line.
            println!("
{}", creds_json(30).replace("test-session-token", &format!("env-{agent}-leaked-{leaked}")));
            std::process::exit(0);
        }
        Ok("sleep") => std::thread::sleep(Duration::from_secs(20)),
        Ok("fail") => {
            eprintln!("the host said no");
            std::process::exit(3);
        }
        _ => {}
    }
}

/// The mode rides in the recipe's own environment, so this process, running the other tests, never sees it.
fn child_spec(mode: &str, timeout: Duration) -> RunSpec {
    RunSpec {
        program: std::env::current_exe().unwrap(),
        args: ["--exact", "identity::aws_broker::mint::tests::recipe_child", "--nocapture", "--test-threads=1"].iter().map(|s| s.to_string()).collect(),
        env: vec![("AGENTMUX_AGENT_ID".to_string(), "jcont".to_string()), ("AWS_TEST_RECIPE_CHILD".to_string(), mode.to_string())],
        timeout,
    }
}

#[test]
fn only_the_hosts_own_aws_setup_and_what_a_program_needs_are_passed_on() {
    for kept in ["PATH", "HOME", "USERPROFILE", "AWS_PROFILE", "AWS_CONFIG_FILE", "HTTPS_PROXY", "SystemRoot"] {
        assert!(passes(kept), "{kept}");
    }
    for dropped in ["AGENTMUX_AUTH_KEY", "AGENTMUX_AGENT_TOKEN", "AGENTMUX_LOCAL_URL", "GH_TOKEN", "ANTHROPIC_API_KEY", "CLAUDE_CONFIG_DIR", "SECRET"] {
        assert!(!passes(dropped), "{dropped}");
    }
}

#[tokio::test]
async fn the_real_runner_gives_the_program_a_clean_environment_and_the_agent_it_is_for() {
    // Whatever srv holds of its own instance identity (set in an agent's shell, where these tests may run) is not passed on.
    let out = HostRunner.run(child_spec("creds", Duration::from_secs(20))).await.ok().unwrap();
    assert!(out.success, "{}", out.stderr);
    let m = parse_process_json(&out.stdout, now()).unwrap();
    assert_eq!(m.session_token.as_str(), "env-jcont-leaked-false");
}

#[tokio::test]
async fn the_real_runner_reports_a_failure_and_stops_a_program_that_takes_too_long() {
    let failed = HostRunner.run(child_spec("fail", Duration::from_secs(20))).await.ok().unwrap();
    assert!(!failed.success && failed.stderr.contains("the host said no"), "{}", failed.stderr);
    let started = std::time::Instant::now();
    let slow = HostRunner.run(child_spec("sleep", Duration::from_millis(300))).await;
    assert!(matches!(slow, Err(RunError::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(10), "it did not wait for the program");
}
