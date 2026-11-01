use std::sync::Arc;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};

use super::*;
use crate::backend::jobs::reporters::REPORTERS_FILE;
use crate::backend::mps::Broker;
use crate::backend::storage::job_runs::reason;
use crate::backend::storage::store::Store;
use crate::backend::widget_signature::test_keys;

const JOB: &str = "acme.nightly";
const NOW: i64 = 1_792_800_000_000;
const DAY: i64 = 24 * 3600 * 1000;

struct Jobs(Vec<CloudJob>);

impl CloudJobs for Jobs {
    fn cloud_job(&self, job_id: &str) -> Option<CloudJob> {
        self.0.iter().find(|j| j.id == job_id).cloned()
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    ledger: Arc<RunLedger>,
    reporters: Reporters,
    jobs: Jobs,
}

/// A ledger with the reporter of seed 1 pinned and an approved cloud job `acme.nightly` that names its key.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());
    let ledger = RunLedger::new(store.clone(), None, Arc::new(Broker::new()));
    let reporters = Reporters::open(&dir.path().join(REPORTERS_FILE));
    reporters.add("acme-ci", &test_keys::public_key(1)).unwrap();
    let jobs = Jobs(vec![CloudJob { id: JOB.into(), name: "Nightly".into(), reporter_key: test_keys::public_key(1) }]);
    Rig { _dir: dir, store, ledger, reporters, jobs }
}

fn body(extra: Value) -> Value {
    let mut b = json!({ "schema": SCHEMA, "event_id": "e1", "job_id": JOB, "run_id": "r1", "phase": "started", "at": NOW });
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

/// `body` signed by the key from `seed`.
fn signed(seed: u8, body: &Value) -> SignedJobEvent {
    let text = body.to_string();
    let ev: JobEvent = serde_json::from_str(&text).unwrap();
    let key = SigningKey::from_bytes(&[seed; 32]);
    let b64 = base64::engine::general_purpose::STANDARD;
    SignedJobEvent { body: text.clone(), public_key: test_keys::public_key(seed), signature: b64.encode(key.sign(&signed_bytes(&ev, &text)).to_bytes()) }
}

fn finished(extra: Value) -> Value {
    let mut b = body(json!({ "phase": "finished", "event_id": "e2", "at": NOW + 61_000, "infra": "ok", "outcome": "ok", "summary": "3 pages checked" }));
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

fn row(rig: &Rig) -> JobRun {
    rig.store.job_run_get(&cloud_run_id(JOB, "r1")).unwrap().expect("the run is in the ledger")
}

fn ingest(rig: &Rig, ev: &SignedJobEvent) -> Result<Applied, Dropped> {
    rig.ledger.ingest_cloud_event(ev, &rig.reporters, &rig.jobs, NOW + 1000)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_and_a_finish_make_one_closed_row_with_the_reporters_figures() {
    let rig = rig();
    assert_eq!(ingest(&rig, &signed(1, &body(json!({})))), Ok(Applied::Started(CloudStart::Opened)));
    let r = row(&rig);
    assert_eq!((r.state.as_str(), r.job_source.as_str(), r.job_name.as_str(), r.trigger.as_str()), ("running", "cloud", "Nightly", "schedule"));
    assert_eq!(r.reporter, crate::backend::widget_signature::fingerprint(&test_keys::public_key(1)));
    let done = finished(json!({ "duration_ms": 61000, "cost_usd": 0.05, "tokens": { "input": 10, "output": 20 }, "findings": 3, "links": [{ "label": "Report", "url": "https://example.com/r/1" }] }));
    assert_eq!(ingest(&rig, &signed(1, &done)), Ok(Applied::Finished(CloudFinish::Closed)));
    let r = row(&rig);
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.outcome.as_str(), r.summary.as_str()), ("finished", "ok", "ok", "3 pages checked"));
    assert_eq!((r.started_at_ms, r.ended_at_ms, r.cost_usd, r.cost_known, r.output_tokens), (NOW, Some(NOW + 61_000), 0.05, true, 20));
    assert_eq!(r.links["findings"], 3);
    assert_eq!(r.links["reported"][0]["url"], "https://example.com/r/1");
    assert_eq!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().len(), 1, "one row for the run");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_event_id_is_written_once() {
    let rig = rig();
    let ev = signed(1, &body(json!({})));
    assert_eq!(ingest(&rig, &ev), Ok(Applied::Started(CloudStart::Opened)));
    assert_eq!(ingest(&rig, &ev), Ok(Applied::Repeat));
    assert_eq!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_event_id_in_another_run_is_its_own_event() {
    let rig = rig();
    assert_eq!(ingest(&rig, &signed(1, &body(json!({})))), Ok(Applied::Started(CloudStart::Opened)));
    let other = signed(1, &body(json!({ "run_id": "r2" })));
    assert_eq!(ingest(&rig, &other), Ok(Applied::Started(CloudStart::Opened)));
    assert_eq!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_the_ledger_could_not_write_can_be_sent_again() {
    let rig = rig();
    let ev = signed(1, &body(json!({})));
    let rename = |from: &str, to: &str| rig.store.conn().lock().unwrap().execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}")).unwrap();
    rename("db_job_runs", "db_job_runs_away");
    assert_eq!(ingest(&rig, &ev), Err(Dropped::LedgerError));
    rename("db_job_runs_away", "db_job_runs");
    assert_eq!(ingest(&rig, &ev), Ok(Applied::Started(CloudStart::Opened)), "the failed write left no trace of the event id");
    assert_eq!(ingest(&rig, &ev), Ok(Applied::Repeat));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_that_is_not_genuine_is_dropped_and_counted() {
    let rig = rig();
    let before: u64 = drops().iter().map(|(_, n)| n).sum();
    let ok = body(json!({}));
    let mut tampered = signed(1, &ok);
    tampered.body = tampered.body.replace("r1", "r2");
    let cases: Vec<(SignedJobEvent, Dropped)> = vec![
        (signed(2, &ok), Dropped::UnknownReporter),
        (tampered, Dropped::BadSignature),
        (SignedJobEvent { signature: String::new(), ..signed(1, &ok) }, Dropped::BadSignature),
        (signed(1, &body(json!({ "job_id": "acme.other" }))), Dropped::UnknownJob),
        (signed(1, &body(json!({ "at": NOW - 8 * DAY }))), Dropped::TooOld),
        (signed(1, &body(json!({ "at": NOW + 2 * 3600 * 1000 }))), Dropped::FromTheFuture),
        (SignedJobEvent { body: "x".repeat(MAX_BODY_BYTES + 1), ..signed(1, &ok) }, Dropped::TooBig),
    ];
    for (ev, why) in cases {
        assert_eq!(ingest(&rig, &ev), Err(why.clone()), "{why:?}");
    }
    assert!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().is_empty(), "nothing became a run");
    assert!(drops().iter().map(|(_, n)| n).sum::<u64>() >= before + 7);
    assert!(drops().iter().any(|(label, _)| *label == "unknown job"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_reporter_cannot_report_for_a_job_that_names_another_key() {
    let rig = rig();
    rig.reporters.add("other-ci", &test_keys::public_key(2)).unwrap();
    assert_eq!(ingest(&rig, &signed(2, &body(json!({})))), Err(Dropped::WrongReporter));
    assert!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_widgets_signature_does_not_pass_as_an_event() {
    let rig = rig();
    let ok = body(json!({}));
    let ev: JobEvent = serde_json::from_str(&ok.to_string()).unwrap();
    // Signed the way a widget is, over the same fields: wrong prefix.
    let key = SigningKey::from_bytes(&[1; 32]);
    let widget_style = crate::backend::widget_signature::signed_message(&crate::backend::widget_signature::WIDGET, &ev.job_id, &ev.run_id, &ev.event_id);
    let b64 = base64::engine::general_purpose::STANDARD;
    let forged = SignedJobEvent { body: ok.to_string(), public_key: test_keys::public_key(1), signature: b64.encode(key.sign(&widget_style).to_bytes()) };
    assert_eq!(ingest(&rig, &forged), Err(Dropped::BadSignature));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finish_with_no_start_makes_a_closed_row_and_a_late_start_changes_nothing() {
    let rig = rig();
    let done = finished(json!({ "duration_ms": 60000 }));
    assert_eq!(ingest(&rig, &signed(1, &done)), Ok(Applied::Finished(CloudFinish::Created)));
    let r = row(&rig);
    assert_eq!((r.state.as_str(), r.started_at_ms, r.ended_at_ms), ("finished", NOW + 1000, Some(NOW + 61_000)));
    // The start arrives after the finish (events can cross): the run stays as it is.
    assert_eq!(ingest(&rig, &signed(1, &body(json!({})))), Ok(Applied::Started(CloudStart::Known)));
    assert_eq!(row(&rig).state, "finished");
    // A second finish for the same run changes nothing.
    assert_eq!(ingest(&rig, &signed(1, &finished(json!({ "event_id": "e3", "outcome": "failed" })))), Ok(Applied::Finished(CloudFinish::Known)));
    assert_eq!(row(&rig).outcome, "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_moves_the_deadline_only_for_a_running_run() {
    let rig = rig();
    let beat = |id: &str, at: i64| signed(1, &body(json!({ "phase": "heartbeat", "event_id": id, "at": at })));
    assert_eq!(ingest(&rig, &beat("h0", NOW)), Ok(Applied::Ignored), "no run yet: nothing to push back");
    ingest(&rig, &signed(1, &body(json!({})))).unwrap();
    assert_eq!(ingest(&rig, &beat("h1", NOW + 30_000)), Ok(Applied::Heartbeat));
    assert_eq!(ingest(&rig, &beat("h2", NOW + 10_000)), Ok(Applied::Heartbeat));
    assert_eq!(row(&rig).links["heartbeat_ms"], NOW + 30_000, "a late-arriving older beat never moves it back");
    ingest(&rig, &signed(1, &finished(json!({})))).unwrap();
    assert_eq!(ingest(&rig, &beat("h3", NOW + 90_000)), Ok(Applied::Ignored));
}

/// The watchdog's `missed` row for `slot`, as PR 5 writes it.
fn given_up(rig: &Rig, slot: i64, state: &str) {
    let mut run = JobRun::new("watchdog-1", JOB, "cloud", "Nightly", "schedule", state, slot);
    run.scheduled_for_ms = Some(slot);
    run.reason = reason::NO_REPORT.to_string();
    run.ended_at_ms = Some(slot + 1);
    assert!(rig.store.job_run_insert(&run).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_event_takes_over_the_missed_row_of_its_slot_and_reads_late() {
    for first in ["finished", "started"] {
        let rig = rig();
        given_up(&rig, NOW - 60_000, "missed");
        let slot = json!({ "scheduled_for": NOW - 60_000 });
        if first == "started" {
            assert_eq!(ingest(&rig, &signed(1, &body(slot.clone()))), Ok(Applied::Started(CloudStart::TookOverMissed)));
            assert_eq!((row(&rig).state.as_str(), row(&rig).reason.as_str()), ("running", "late"));
        }
        let done = finished(slot);
        let expected = if first == "started" { CloudFinish::Closed } else { CloudFinish::Late };
        assert_eq!(ingest(&rig, &signed(1, &done)), Ok(Applied::Finished(expected)));
        let r = row(&rig);
        assert_eq!(r.state, "finished");
        assert_eq!(r.reason, "late", "{first}");
        assert_eq!(rig.store.job_runs_for_job(JOB, 10, None).unwrap().len(), 1, "the slot is one row, not a miss and a run");
        assert_eq!(r.scheduled_for_ms, Some(NOW - 60_000), "the slot's lease stays with it");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_run_that_finishes_late_reads_late() {
    let rig = rig();
    ingest(&rig, &signed(1, &body(json!({})))).unwrap();
    // The watchdog gave it up.
    rig.store.conn().lock().unwrap().execute("UPDATE db_job_runs SET state = 'lost', ended_at_ms = ?1", [NOW + 5000]).unwrap();
    assert_eq!(ingest(&rig, &signed(1, &finished(json!({})))), Ok(Applied::Finished(CloudFinish::Late)));
    let r = row(&rig);
    assert_eq!((r.state.as_str(), r.reason.as_str(), r.outcome.as_str()), ("finished", "late", "ok"));
}

#[test]
fn the_event_rules_of_the_public_schema() {
    let ok = |extra: Value| parse(&body(extra).to_string()).map(|_| ());
    assert!(ok(json!({})).is_ok());
    for (what, extra) in [
        ("a field the schema doesn't have", json!({ "colour": "red" })),
        ("another schema version", json!({ "schema": "agentmux.job-event/2" })),
        ("an event id with a space", json!({ "event_id": "a b" })),
        ("a job id with two dots", json!({ "job_id": "a.b.c" })),
        ("an uppercase job id", json!({ "job_id": "Acme.nightly" })),
        ("a long summary", json!({ "summary": "x".repeat(301) })),
        ("a negative cost", json!({ "cost_usd": -1.0 })),
        ("an http link", json!({ "links": [{ "label": "r", "url": "http://example.com" }] })),
        ("a javascript link", json!({ "links": [{ "label": "r", "url": "javascript:alert(1)" }] })),
        ("six links", json!({ "links": (0..6).map(|i| json!({ "label": "r", "url": format!("https://example.com/{i}") })).collect::<Vec<_>>() })),
        ("an unknown phase", json!({ "phase": "paused" })),
        ("a finish with no outcome", json!({ "phase": "finished", "infra": "ok" })),
    ] {
        assert!(matches!(ok(extra), Err(Dropped::Malformed(_))), "{what}");
    }
    assert!(ok(json!({ "summary": "x".repeat(300), "links": [{ "label": "r", "url": "https://example.com/a" }] })).is_ok());
}

/// The published schema names the same fields, values and limits the code reads.
#[test]
fn the_published_schema_matches_the_code() {
    let schema: Value = serde_json::from_str(include_str!("../../../../../schema/agentmux-job-event.v1.schema.json")).unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], SCHEMA);
    let strings = |pointer: &str| -> Vec<String> {
        let mut v: Vec<String> = schema.pointer(pointer).unwrap_or_else(|| panic!("{pointer}")).as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
        v.sort();
        v
    };
    let sorted = |xs: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = xs.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(strings("/properties/phase/enum"), sorted(&["started", "finished", "heartbeat"]));
    assert_eq!(strings("/properties/trigger/enum"), sorted(&["schedule", "event", "manual", "retry"]));
    assert_eq!(strings("/properties/infra/enum"), sorted(&["ok", "error", "timeout"]));
    assert_eq!(strings("/properties/outcome/enum"), sorted(&["ok", "findings", "failed", "unknown"]));
    assert_eq!(strings("/required"), sorted(&["schema", "event_id", "job_id", "run_id", "phase", "at"]));
    let mut fields: Vec<String> = schema["properties"].as_object().unwrap().keys().cloned().collect();
    fields.sort();
    assert_eq!(
        fields,
        sorted(&["schema", "event_id", "job_id", "run_id", "phase", "at", "scheduled_for", "trigger", "attempt", "infra", "outcome", "summary", "duration_ms", "cost_usd", "tokens", "findings", "links"])
    );
    assert_eq!(schema["properties"]["summary"]["maxLength"], SUMMARY_MAX_CHARS);
    assert_eq!(schema["properties"]["links"]["maxItems"], MAX_LINKS);
    // Every field the schema has is one the code takes (deny_unknown_fields would refuse the rest).
    let all = json!({
        "schema": SCHEMA, "event_id": "e", "job_id": "a.b", "run_id": "r", "phase": "finished", "at": 1, "scheduled_for": 1, "trigger": "retry", "attempt": 2,
        "infra": "timeout", "outcome": "findings", "summary": "s", "duration_ms": 1, "cost_usd": 0.1, "tokens": { "input": 1, "output": 1 }, "findings": 1,
        "links": [{ "label": "l", "url": "https://example.com" }]
    });
    assert_eq!(all.as_object().unwrap().len(), schema["properties"].as_object().unwrap().len());
    parse(&all.to_string()).unwrap();
}
