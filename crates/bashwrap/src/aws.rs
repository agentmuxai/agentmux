// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AWS credentials for a container agent's SDK or CLI, as AWS's standard `credential_process`.
//! Spec: docs/specs/SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md §12.
//!
//! `aws-credentials --profile <grant>` is what each profile of the container's AWS config runs (srv writes the
//! config). It asks `POST /api/v1/aws/credentials`; who is asking is decided by srv from the agent's token
//! (`AGENTMUX_AGENT_TOKEN`), never from the request. The reason for a write grant rides in `AGENTMUX_AWS_REASON`
//! (the SDK process inherits it from the agent's command line), and an optional lifetime in `AGENTMUX_AWS_DURATION`.
//!
//! Stdout is the credentials as `credential_process` JSON on success and empty on every failure, so a broken answer
//! can never be read as credentials. A failure is one sentence on stderr, which the AWS CLI shows after "Error when
//! retrieving credentials from custom-process".

use std::io::Write;
use std::time::Duration;

use serde_json::{json, Value};

use crate::mps_client::WpsClient;

/// srv may wait on the host's AWS for 25 seconds; a stuck call must not hold a deploy forever.
const ASK_TIMEOUT: Duration = Duration::from_secs(30);

/// What the route answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// The `credential_process` JSON, ready to print.
    Credentials(String),
    /// The sentence to show.
    Refused(String),
}

/// The request body: the grant, and what the agent said about why and for how long, if anything.
pub fn request_body(profile: &str, reason: Option<&str>, duration: Option<&str>) -> Value {
    let mut body = json!({ "profile": profile });
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        body["reason"] = json!(reason);
    }
    if let Some(seconds) = duration.and_then(|d| d.trim().parse::<u64>().ok()) {
        body["duration_seconds"] = json!(seconds);
    }
    body
}

/// srv's answer as the SDK should see it: only the five fields of `credential_process`, and only when all are there.
pub fn answer_of(status: u16, body: &Value) -> Answer {
    if (200..300).contains(&status) {
        let text = |key: &str| body.get(key).and_then(Value::as_str).filter(|v| !v.is_empty());
        return match (text("AccessKeyId"), text("SecretAccessKey"), text("SessionToken"), text("Expiration")) {
            (Some(id), Some(secret), Some(token), Some(expiration)) => Answer::Credentials(
                json!({ "Version": 1, "AccessKeyId": id, "SecretAccessKey": secret, "SessionToken": token, "Expiration": expiration }).to_string(),
            ),
            _ => Answer::Refused("AgentMux answered, but not with credentials".to_string()),
        };
    }
    Answer::Refused(body.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("AgentMux refused ({status})")))
}

/// Ask srv for the grant `profile`.
pub fn ask(profile: &str) -> Answer {
    let Some(client) = WpsClient::from_env() else {
        return Answer::Refused("AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY aren't set: run this from an agent's own shell".to_string());
    };
    let agent_token = std::env::var("AGENTMUX_AGENT_TOKEN").ok();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Answer::Refused(format!("couldn't start: {e}")),
    };
    let reason = std::env::var("AGENTMUX_AWS_REASON").ok();
    let duration = std::env::var("AGENTMUX_AWS_DURATION").ok();
    let body = request_body(profile, reason.as_deref(), duration.as_deref());
    match rt.block_on(client.post_json_status("/api/v1/aws/credentials", agent_token.as_deref(), &body, ASK_TIMEOUT)) {
        Ok((status, answer)) => answer_of(status, &answer),
        Err(e) => Answer::Refused(format!("couldn't reach AgentMux: {e}")),
    }
}

/// `aws-credentials --profile <name>`: exit 0 with the JSON on stdout, or exit 1 with a sentence on stderr.
pub fn run_aws_credentials(profile: &str) -> i32 {
    match ask(profile) {
        Answer::Credentials(json) => {
            let mut out = std::io::stdout();
            let _ = out.write_all(json.as_bytes());
            let _ = out.write_all(b"\n");
            let _ = out.flush();
            0
        }
        Answer::Refused(why) => {
            eprintln!("{why}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDS: &str = r#"{"Version":1,"AccessKeyId":"AKIATEST","SecretAccessKey":"sekret","SessionToken":"tok","Expiration":"2026-10-31T13:00:00Z","Extra":"dropped"}"#;

    #[test]
    fn the_request_names_the_grant_and_carries_the_reason_and_lifetime_only_when_given() {
        assert_eq!(request_body("deploy", None, None), json!({ "profile": "deploy" }));
        assert_eq!(request_body("deploy", Some("  "), Some("soon")), json!({ "profile": "deploy" }));
        assert_eq!(
            request_body("deploy", Some(" deploy stack X "), Some("1800")),
            json!({ "profile": "deploy", "reason": "deploy stack X", "duration_seconds": 1800 })
        );
    }

    #[test]
    fn credentials_are_passed_on_as_the_five_fields_and_nothing_else() {
        let Answer::Credentials(out) = answer_of(200, &serde_json::from_str(CREDS).unwrap()) else { panic!("credentials") };
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["Version"], 1);
        assert_eq!(parsed["SessionToken"], "tok");
        assert!(parsed.get("Extra").is_none(), "{out}");
        assert!(!out.contains('\n'), "one line");
    }

    #[test]
    fn an_answer_missing_a_field_is_never_credentials() {
        for key in ["AccessKeyId", "SecretAccessKey", "SessionToken", "Expiration"] {
            let mut body: Value = serde_json::from_str(CREDS).unwrap();
            body.as_object_mut().unwrap().remove(key);
            assert!(matches!(answer_of(200, &body), Answer::Refused(_)), "{key}");
        }
        assert!(matches!(answer_of(200, &json!({ "AccessKeyId": "" })), Answer::Refused(_)));
    }

    #[test]
    fn a_refusal_is_srvs_own_sentence() {
        let body = json!({ "error": "REASON_REQUIRED: The grant 'deploy' needs a reason." });
        assert_eq!(answer_of(400, &body), Answer::Refused("REASON_REQUIRED: The grant 'deploy' needs a reason.".to_string()));
        assert_eq!(answer_of(502, &json!({})), Answer::Refused("AgentMux refused (502)".to_string()));
        // Whatever it says, a refusal never prints credentials.
        assert!(matches!(answer_of(403, &serde_json::from_str(CREDS).unwrap()), Answer::Refused(_)));
    }

    #[test]
    fn the_reason_the_agents_token_and_no_name_reach_srv() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, srv) = crate::github::tests::stub_srv(200, CREDS);
        std::env::set_var("AGENTMUX_LOCAL_URL", &url);
        std::env::set_var("AGENTMUX_AUTH_KEY", "key");
        std::env::set_var("AGENTMUX_AGENT_TOKEN", "agent-token-1");
        std::env::set_var("AGENTMUX_AWS_REASON", "deploy stack X");
        std::env::set_var("AGENTMUX_AWS_DURATION", "1800");
        let answer = ask("deploy");
        std::env::remove_var("AGENTMUX_AWS_REASON");
        std::env::remove_var("AGENTMUX_AWS_DURATION");
        let seen = srv.join().unwrap();
        assert!(matches!(&answer, Answer::Credentials(c) if c.contains("AKIATEST")));
        assert!(seen.starts_with("POST /api/v1/aws/credentials "), "{seen}");
        assert!(seen.to_ascii_lowercase().contains("x-agent-token: agent-token-1"), "who is asking is the agent's own token: {seen}");
        assert!(seen.contains(r#""profile":"deploy""#) && seen.contains(r#""reason":"deploy stack X""#) && seen.contains(r#""duration_seconds":1800"#), "{seen}");
        assert!(!seen.contains("agent_id"), "no name in the request: srv decides who is asking");
    }

    #[test]
    fn a_refused_request_prints_nothing_for_the_sdk_to_read() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, srv) = crate::github::tests::stub_srv(403, r#"{"error":"AWS_NOT_SHARED: AWS isn't shared with container agents on this machine."}"#);
        std::env::set_var("AGENTMUX_LOCAL_URL", &url);
        std::env::set_var("AGENTMUX_AUTH_KEY", "key");
        std::env::remove_var("AGENTMUX_AGENT_TOKEN");
        std::env::remove_var("AGENTMUX_AWS_REASON");
        let answer = ask("read");
        srv.join().unwrap();
        assert_eq!(answer, Answer::Refused("AWS_NOT_SHARED: AWS isn't shared with container agents on this machine.".to_string()));
    }
}
