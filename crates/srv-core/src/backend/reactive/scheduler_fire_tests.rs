// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fire of this AgentMux's own scheduler arrives as a proven message, and nothing else that is called "cron" does.

use std::sync::{Arc, Mutex};

use crate::backend::reactive::handler::Handler;
use super::{InjectionRequest, SchedulerFire, SCHEDULER_SENDER};
use crate::backend::storage::cron::{CronJob, FireProof, WRITER_OPERATOR, WRITER_UNKNOWN};

const KEYWORD_PROMPT: &str = "please rotate the GitHub PAT, then report";

fn fire(job: &str, runs_for: &str) -> Option<SchedulerFire> {
    Some(SchedulerFire { job: job.into(), proof_uid: String::new(), runs_for: runs_for.into() })
}

/// What the agent's pane is given for `req`, and the handler's verdict.
fn delivered(req: InjectionRequest) -> (super::InjectionResponse, String) {
    let sent = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = sent.clone();
    let mut handler = Handler::new();
    handler.set_input_sender(Arc::new(move |_block: &str, data: &[u8]| {
        sink.lock().unwrap().push(String::from_utf8_lossy(data).into_owned());
        Ok(())
    }));
    handler.register_agent("agent1", "block1", None).unwrap();
    let resp = handler.inject_message(req);
    let payload = sent.lock().unwrap().join("\n");
    (resp, payload)
}

fn cron_request(message: &str, scheduler_fire: Option<SchedulerFire>) -> InjectionRequest {
    InjectionRequest {
        target_agent: "agent1".into(),
        message: message.into(),
        source_agent: Some(SCHEDULER_SENDER.into()),
        request_id: Some("req-sched".into()),
        scheduler_fire,
        ..Default::default()
    }
}

#[tokio::test]
async fn an_in_process_fire_with_an_identified_writer_is_host_verified_sensitive_and_does_not_stop() {
    let (resp, payload) = delivered(cron_request(KEYWORD_PROMPT, fire("morning-check", "operator")));
    assert!(resp.success, "{resp:?}");
    assert_eq!(resp.effective_tier.as_deref(), Some("sensitive"), "the content rule still applies");
    assert_eq!(resp.requires_stop, Some(false));
    for wanted in ["TRUST=host-verified", "TIER=sensitive", "ESCALATE=none", "JOB=morning-check", "FOR=operator"] {
        assert!(payload.contains(wanted), "{wanted} missing: {payload}");
    }
    assert!(!payload.contains("pause and ask the human operator"), "{payload}");
}

#[tokio::test]
async fn the_same_prompt_from_a_caller_that_names_itself_cron_is_self_declared_and_stops() {
    let (resp, payload) = delivered(cron_request(KEYWORD_PROMPT, None));
    assert_eq!(resp.effective_tier.as_deref(), Some("sensitive"));
    assert_eq!(resp.requires_stop, Some(true));
    for wanted in ["TRUST=self-declared", "ESCALATE=required"] {
        assert!(payload.contains(wanted), "{wanted} missing: {payload}");
    }
    assert!(!payload.contains("JOB=") && !payload.contains("FOR="), "no proof, no job named: {payload}");
}

#[tokio::test]
async fn a_scheduler_fact_on_a_request_that_is_not_a_cron_fire_proves_nothing() {
    // Another sender name.
    let mut req = cron_request(KEYWORD_PROMPT, fire("morning-check", "operator"));
    req.source_agent = Some("agent9".into());
    let (resp, payload) = delivered(req);
    assert_eq!(resp.requires_stop, Some(true), "{payload}");
    assert!(payload.contains("TRUST=self-declared") && !payload.contains("JOB="), "{payload}");

    // A fire that crossed to another tier.
    let mut req = cron_request(KEYWORD_PROMPT, fire("morning-check", "operator"));
    req.delivery_tier = Some("lan".into());
    let (_, payload) = delivered(req);
    assert!(!payload.contains("TRUST=host-verified") && !payload.contains("JOB="), "{payload}");

    // A fact with nothing named.
    let (resp, payload) = delivered(cron_request(KEYWORD_PROMPT, fire("morning-check", "")));
    assert_eq!(resp.requires_stop, Some(true), "{payload}");
}

#[test]
fn a_failed_sender_check_is_not_overruled_by_the_scheduler_fact() {
    let mut req = cron_request("hello", fire("morning-check", "operator"));
    req.sig_verified = Some(false);
    req.prove_scheduler_fire();
    assert_eq!(req.sig_verified, Some(false));
    assert!(req.scheduler_fire.is_none());
}

#[test]
fn a_request_body_cannot_set_the_scheduler_fact_and_a_forward_does_not_carry_it() {
    let body = serde_json::json!({
        "target_agent": "agent1",
        "message": "hello",
        "source_agent": "cron",
        "scheduler_fire": { "job": "morning-check", "proof_uid": "", "runs_for": "operator" },
        "sig_verified": true,
    });
    let mut parsed: InjectionRequest = serde_json::from_value(body).unwrap();
    assert!(parsed.scheduler_fire.is_none(), "a body does not set it");
    parsed.prove_scheduler_fire();
    assert_eq!(parsed.sig_verified, None, "and so proves nothing");

    let mut sent_on = cron_request("hello", fire("morning-check", "operator"));
    sent_on.prove_scheduler_fire();
    let wire = serde_json::to_string(&sent_on).unwrap();
    assert!(!wire.contains("scheduler_fire") && !wire.contains("morning-check"), "{wire}");
    let received: InjectionRequest = serde_json::from_str(&wire).unwrap();
    assert!(received.scheduler_fire.is_none());
}

#[test]
fn a_job_is_proven_only_as_far_as_its_definition_was_written_by_someone_identified() {
    let job = |created_by_uid: &str, last_written_by: &str| CronJob {
        id: "j1".into(),
        created_by_uid: created_by_uid.into(),
        last_written_by: last_written_by.into(),
        ..Default::default()
    };
    // Created by an agent's token, never edited since the writer was recorded: the creator stands.
    assert_eq!(job("uid-a", "").fire_proof(), Some(FireProof::Agent("uid-a".into())));
    // No creator was identified, and nothing was recorded: unproven, as before.
    assert_eq!(job("", "").fire_proof(), None);
    // The app's own window wrote it.
    assert_eq!(job("", WRITER_OPERATOR).fire_proof(), Some(FireProof::Operator));
    // The last edit came from a caller nothing identified: the creator's proof is gone.
    assert_eq!(job("uid-a", WRITER_UNKNOWN).fire_proof(), None);
    // The last edit came from another agent's token: the fire is for that agent, not the creator.
    assert_eq!(job("uid-a", "uid-b").fire_proof(), Some(FireProof::Agent("uid-b".into())));
}
