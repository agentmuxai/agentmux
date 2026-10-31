// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cloud jobs report in: the signed run event (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.1, §4.2).
//!
//! A cloud job posts a small event when a run starts, while it runs, and when it finishes. Each arrives as an
//! envelope: the event's JSON body as the reporter wrote it, the reporter's public key, and an Ed25519 signature
//! over `agentmux-job-event-v1\0<job_id>\0<run_id>\0<event_id>\0<hex SHA-256 of the body>`. An event counts only if
//! its key is one the operator pinned ([`super::reporters`]), its signature holds, and an approved cloud job
//! manifest names that job with that key. Anything else is dropped and counted ([`drops`]), never shown as a run.
//!
//! The text in an event (`summary`, link labels) is data: it is clipped here and shown as text, never an
//! instruction.

use std::collections::BTreeMap;
use std::sync::Mutex;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::reporters::Reporters;
use super::RunLedger;
use crate::backend::storage::job_cloud::{cloud_run_id, CloudFinish, CloudStart};
use crate::backend::storage::job_runs::{clip_summary, infra, state, JobRun, SUMMARY_MAX_CHARS};

/// The schema name every event carries.
pub const SCHEMA: &str = "agentmux.job-event/1";
/// The bytes an Ed25519 signature over an event starts with, so it can't pass as a widget's or a template's.
pub const SIGNED_PREFIX: &[u8] = b"agentmux-job-event-v1\0";
/// The most a body may be.
pub const MAX_BODY_BYTES: usize = 8 * 1024;
/// Events older than this are refused (§4.2).
pub const MAX_AGE_MS: i64 = 7 * 24 * 3600 * 1000;
/// How far ahead of this computer's clock an event may claim to be, for a reporter whose clock runs fast.
pub const MAX_AHEAD_MS: i64 = 3600 * 1000;
const MAX_LINKS: usize = 5;

/// One event as it arrives: the body exactly as the reporter signed it, the key, the signature.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SignedJobEvent {
    pub body: String,
    /// The reporter's public key, base64.
    pub public_key: String,
    /// The Ed25519 signature, base64.
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Started,
    Finished,
    Heartbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Schedule,
    Event,
    Manual,
    Retry,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Trigger::Schedule => "schedule",
            Trigger::Event => "event",
            Trigger::Manual => "manual",
            Trigger::Retry => "retry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Infra {
    Ok,
    Error,
    Timeout,
}

impl Infra {
    fn as_str(self) -> &'static str {
        match self {
            Infra::Ok => infra::OK,
            Infra::Error => infra::ERROR,
            Infra::Timeout => infra::TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Findings,
    Failed,
    Unknown,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Findings => "findings",
            Outcome::Failed => "failed",
            Outcome::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventLink {
    pub label: String,
    pub url: String,
}

/// The event body (`schema/agentmux-job-event.v1.schema.json`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobEvent {
    pub schema: String,
    pub event_id: String,
    pub job_id: String,
    pub run_id: String,
    pub phase: Phase,
    pub at: i64,
    #[serde(default)]
    pub scheduled_for: Option<i64>,
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default)]
    pub attempt: Option<i64>,
    #[serde(default)]
    pub infra: Option<Infra>,
    #[serde(default)]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub findings: Option<i64>,
    #[serde(default)]
    pub links: Option<Vec<EventLink>>,
}

/// Why an event was not written. The label is what the Jobs tab's diagnostic line says ("3 events dropped:
/// unknown job").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    TooBig,
    Malformed(String),
    TooOld,
    FromTheFuture,
    UnknownReporter,
    BadSignature,
    UnknownJob,
    WrongReporter,
}

impl Dropped {
    pub fn label(&self) -> &'static str {
        match self {
            Dropped::TooBig => "too big",
            Dropped::Malformed(_) => "malformed",
            Dropped::TooOld => "too old",
            Dropped::FromTheFuture => "clock ahead",
            Dropped::UnknownReporter => "unknown reporter",
            Dropped::BadSignature => "bad signature",
            Dropped::UnknownJob => "unknown job",
            Dropped::WrongReporter => "reporter not allowed for this job",
        }
    }
}

/// What a valid event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Started(CloudStart),
    Finished(CloudFinish),
    /// The run's lost deadline moved back.
    Heartbeat,
    /// A heartbeat for a run that isn't running: nothing to push back.
    Ignored,
    /// This event id was written already.
    Repeat,
}

/// An approved cloud job (§4.3): which key may report for it, and its name for the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudJob {
    pub id: String,
    pub name: String,
    /// The public key (base64) whose events count for this job.
    pub reporter_key: String,
}

/// Where approved cloud jobs are found.
pub trait CloudJobs {
    fn cloud_job(&self, job_id: &str) -> Option<CloudJob>;
}

static DROPS: Mutex<BTreeMap<&'static str, u64>> = Mutex::new(BTreeMap::new());

fn count_drop(why: &Dropped) {
    *DROPS.lock().unwrap_or_else(|e| e.into_inner()).entry(why.label()).or_default() += 1;
}

/// How many events were dropped since this AgentMux started, by label.
pub fn drops() -> Vec<(&'static str, u64)> {
    DROPS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(k, v)| (*k, *v)).collect()
}

fn word(s: &str, max: usize, extra: &str) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.as_bytes().contains(&b))
}

/// `publisher.name`: lowercase letters, digits and `-`, one dot.
fn is_job_id(s: &str) -> bool {
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    s.len() <= 80 && s.split_once('.').is_some_and(|(a, b)| ok(a) && ok(b))
}

/// Parse and check one event body.
pub fn parse(body: &str) -> Result<JobEvent, Dropped> {
    if body.len() > MAX_BODY_BYTES {
        return Err(Dropped::TooBig);
    }
    let ev: JobEvent = serde_json::from_str(body).map_err(|e| Dropped::Malformed(e.to_string()))?;
    let bad = |why: &str| Err(Dropped::Malformed(why.to_string()));
    if ev.schema != SCHEMA {
        return bad("schema");
    }
    if !word(&ev.event_id, 64, "-_") {
        return bad("event_id");
    }
    if !is_job_id(&ev.job_id) {
        return bad("job_id");
    }
    if !word(&ev.run_id, 128, "._:-") {
        return bad("run_id");
    }
    if ev.at <= 0 || ev.scheduled_for.is_some_and(|s| s <= 0) || ev.attempt.is_some_and(|a| !(1..=1000).contains(&a)) {
        return bad("at, scheduled_for or attempt");
    }
    if ev.phase == Phase::Finished && (ev.infra.is_none() || ev.outcome.is_none()) {
        return bad("a finished event needs infra and outcome");
    }
    if ev.summary.as_deref().is_some_and(|s| s.chars().count() > SUMMARY_MAX_CHARS) {
        return bad("summary is over 300 characters");
    }
    if ev.duration_ms.is_some_and(|d| d < 0) || ev.findings.is_some_and(|f| f < 0) || ev.tokens.as_ref().is_some_and(|t| t.input < 0 || t.output < 0) {
        return bad("a negative figure");
    }
    if ev.cost_usd.is_some_and(|c| !c.is_finite() || c < 0.0) {
        return bad("cost_usd");
    }
    if let Some(links) = &ev.links {
        let https = |u: &str| u.len() <= 500 && u.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace));
        if links.len() > MAX_LINKS || links.iter().any(|l| l.label.chars().count() > 80 || !https(&l.url)) {
            return bad("links: at most 5, https only");
        }
    }
    Ok(ev)
}

/// The bytes the reporter signed for `ev`'s `body`.
pub fn signed_bytes(ev: &JobEvent, body: &str) -> Vec<u8> {
    let mut m = SIGNED_PREFIX.to_vec();
    for part in [ev.job_id.as_str(), ev.run_id.as_str(), ev.event_id.as_str()] {
        m.extend_from_slice(part.as_bytes());
        m.push(0);
    }
    m.extend_from_slice(hex(&Sha256::digest(body.as_bytes())).as_bytes());
    m
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn verify(envelope: &SignedJobEvent, ev: &JobEvent) -> Result<(), Dropped> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64.decode(envelope.public_key.trim()).ok().and_then(|k| k.try_into().ok()).ok_or(Dropped::BadSignature)?;
    let signature: [u8; 64] = b64.decode(envelope.signature.trim()).ok().and_then(|s| s.try_into().ok()).ok_or(Dropped::BadSignature)?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| Dropped::BadSignature)?;
    key.verify(&signed_bytes(ev, &envelope.body), &Signature::from_bytes(&signature)).map_err(|_| Dropped::BadSignature)
}

/// The ledger row an event describes: `running` for a start, `finished` for a finish.
fn row_for(ev: &JobEvent, job: &CloudJob, reporter: &str, recorded_by: &str) -> JobRun {
    let finished = ev.phase == Phase::Finished;
    let mut run = JobRun::new(
        &cloud_run_id(&ev.job_id, &ev.run_id),
        &ev.job_id,
        "cloud",
        &job.name,
        ev.trigger.map_or("schedule", Trigger::as_str),
        if finished { state::FINISHED } else { state::RUNNING },
        ev.at,
    );
    run.attempt = ev.attempt.unwrap_or(1);
    run.reporter = reporter.to_string();
    run.recorded_by = recorded_by.to_string();
    let mut links = serde_json::json!({ "run_id": ev.run_id });
    if let Some(slot) = ev.scheduled_for {
        links["scheduled_for_ms"] = slot.into();
    }
    if finished {
        run.infra = ev.infra.map_or("", Infra::as_str).to_string();
        run.outcome = ev.outcome.map_or("unknown", Outcome::as_str).to_string();
        run.summary = clip_summary(ev.summary.as_deref().unwrap_or(""));
        run.ended_at_ms = Some(ev.at);
        // A closed row whose `started` never arrived starts as long before it ended as the run says it lasted.
        run.started_at_ms = ev.at - ev.duration_ms.unwrap_or(0).max(0);
        if let Some(cost) = ev.cost_usd {
            run.cost_usd = cost;
            run.cost_known = true;
        }
        if let Some(t) = &ev.tokens {
            run.output_tokens = t.output;
            links["input_tokens"] = t.input.into();
        }
        if let Some(f) = ev.findings {
            links["findings"] = f.into();
        }
        if let Some(d) = ev.duration_ms {
            links["duration_ms"] = d.into();
        }
        if let Some(l) = &ev.links {
            links["reported"] = l.iter().map(|l| serde_json::json!({ "label": l.label, "url": l.url })).collect();
        }
    }
    run.links = links;
    run
}

impl RunLedger {
    /// Write one cloud event into the ledger, if it is genuine: its key pinned, its signature good, its job approved
    /// with that key, and not seen before. A refused event is counted ([`drops`]) and logged, not shown as a run.
    pub fn ingest_cloud_event(
        &self,
        envelope: &SignedJobEvent,
        reporters: &Reporters,
        jobs: &dyn CloudJobs,
        now_ms: i64,
    ) -> Result<Applied, Dropped> {
        let result = self.apply_cloud_event(envelope, reporters, jobs, now_ms);
        if let Err(why) = &result {
            count_drop(why);
            tracing::info!(target: "jobs", reason = why.label(), detail = ?why, "jobs: a cloud job event was dropped");
        }
        result
    }

    fn apply_cloud_event(&self, envelope: &SignedJobEvent, reporters: &Reporters, jobs: &dyn CloudJobs, now_ms: i64) -> Result<Applied, Dropped> {
        let ev = parse(&envelope.body)?;
        if now_ms - ev.at > MAX_AGE_MS {
            return Err(Dropped::TooOld);
        }
        if ev.at - now_ms > MAX_AHEAD_MS {
            return Err(Dropped::FromTheFuture);
        }
        let fingerprint = reporters.fingerprint_of_pinned(&envelope.public_key).ok_or(Dropped::UnknownReporter)?;
        verify(envelope, &ev)?;
        let job = jobs.cloud_job(&ev.job_id).ok_or(Dropped::UnknownJob)?;
        if job.reporter_key != envelope.public_key {
            return Err(Dropped::WrongReporter);
        }
        let store = self.store();
        let log = |e: crate::backend::storage::StoreError| {
            tracing::warn!(target: "jobs", error = %e, job = %ev.job_id, "jobs: couldn't write a cloud job event");
            Dropped::Malformed("the ledger couldn't be written".to_string())
        };
        if !store.job_event_record(&ev.event_id, &ev.job_id, now_ms).map_err(log)? {
            return Ok(Applied::Repeat);
        }
        let run = row_for(&ev, &job, &fingerprint, &self.recorded_by);
        let applied = match ev.phase {
            Phase::Started => Applied::Started(store.job_cloud_start(&run, ev.scheduled_for).map_err(log)?),
            Phase::Finished => Applied::Finished(store.job_cloud_finish(&run, ev.scheduled_for).map_err(log)?),
            Phase::Heartbeat => {
                if store.job_cloud_heartbeat(&run.id, ev.at).map_err(log)? {
                    Applied::Heartbeat
                } else {
                    Applied::Ignored
                }
            }
        };
        if applied != Applied::Ignored {
            self.changed(&ev.job_id);
        }
        Ok(applied)
    }
}

#[cfg(test)]
#[path = "cloud_events_tests.rs"]
mod tests;
