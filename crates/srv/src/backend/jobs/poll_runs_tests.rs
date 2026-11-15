// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Poll: which questions a job may ask, and a job's run starting for each item that is new in the answer
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2).

use super::*;
use crate::backend::cron::ActionOutcome;
use crate::backend::jobs::definition::trigger_kind;
use crate::backend::jobs::poll::{self, Check, PollState};
use crate::backend::storage::cron::CronJob;

fn filter(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn a_poll_filter_names_a_query_and_how_often_and_only_that_querys_own_keys() {
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 15 }))).is_ok());
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 1, "holder": "agent2", "state": "todo", "pr": "https://example.org/pr/1", "limit": 10, "include_existing": true }))).is_ok());
    assert!(poll::validate_filter(&filter(json!({ "query": "jobs", "every_minutes": 1440, "job": "nightly", "limit": 5 }))).is_ok());
    let all_jobs = poll::validate_filter(&filter(json!({ "query": "jobs", "every_minutes": 5 }))).unwrap_err();
    assert!(all_jobs.contains("has job"), "a poll of every job's newest run would lose runs: {all_jobs}");
    assert!(poll::validate_filter(&filter(json!({ "query": "jobs", "every_minutes": 5, "job": "  " }))).is_err());
    assert!(poll::validate_filter(&filter(json!({ "query": "agents", "every_minutes": 5 }))).is_ok());
    assert!(poll::validate_filter(&filter(json!({ "query": "lan_peers", "every_minutes": 5 }))).is_ok());

    let no_query = poll::validate_filter(&filter(json!({ "every_minutes": 5 }))).unwrap_err();
    assert!(no_query.contains("names its query") && no_query.contains("tasks, jobs, agents, lan_peers"), "{no_query}");
    assert!(poll::validate_filter(&filter(json!({ "query": "muxbus", "every_minutes": 5 }))).unwrap_err().contains("names its query"), "a question with no items to be new");
    for every in [json!(0), json!(1441), json!(2.5), json!("5"), json!(null)] {
        let e = poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": every }))).unwrap_err();
        assert!(e.contains("every_minutes"), "{every}: {e}");
    }
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks" }))).unwrap_err().contains("every_minutes"), "how often is required");
    let stray = poll::validate_filter(&filter(json!({ "query": "agents", "every_minutes": 5, "holder": "x" }))).unwrap_err();
    assert!(stray.contains("no filter key \"holder\"") && stray.contains("query, every_minutes, include_existing"), "{stray}");
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 5, "job": "x" }))).is_err(), "another query's key");
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 5, "limit": 0 }))).is_err());
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 5, "limit": 201 }))).is_err());
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 5, "include_existing": "yes" }))).is_err());
    assert!(poll::validate_filter(&filter(json!({ "query": "tasks", "every_minutes": 5, "holder": 3 }))).is_err(), "text keys are text");
}

fn poll_job(rig: &Rig, id: &str, ask: serde_json::Value) -> CronJob {
    let mut job = test_job(id, "", "", "", "");
    job.created_by = BY_OPERATOR.to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "poll", "filter": ask, "max_per_hour": 1000 });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    rig.store.cron_create(&job).unwrap();
    job
}

/// A check that answers with whatever `answer` holds, and counts what it was asked.
fn fake(answer: &Arc<Mutex<serde_json::Value>>, asked: &Arc<Mutex<Vec<serde_json::Value>>>) -> Check {
    let (answer, asked) = (answer.clone(), asked.clone());
    Arc::new(move |ask| {
        asked.lock().unwrap().push(serde_json::Value::Object(ask));
        let reply = answer.lock().unwrap().clone();
        Box::pin(async move { if reply.is_null() { Err("not ready".to_string()) } else { Ok(reply) } })
    })
}

fn tasks(nums: &[u32]) -> serde_json::Value {
    json!({ "count": nums.len(), "tasks": nums.iter().map(|n| json!({ "num": n, "title": format!("task {n}"), "state": "todo" })).collect::<Vec<_>>() })
}

fn allow_runs(rig: &Rig) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let inputs: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let (ledger_for_runs, sink) = (rig.ledger.clone(), inputs.clone());
    rig.sched.install_actions(Arc::new(move |_, run_id| {
        sink.lock().unwrap().push(ledger_for_runs.event_input(&run_id).unwrap_or(serde_json::Value::Null));
        ledger_for_runs.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        // The run ends at once, so the job's overlap policy lets the next item start (a poll's fan-out is the job's policy).
        ledger_for_runs.drone_finished(&run_id, true, &Default::default(), "");
        Box::pin(async { ActionOutcome::Drone })
    }));
    inputs
}

const MIN: i64 = 60_000;

/// Turning a poll on takes note of what is there; each task that appears afterwards starts one run, once.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_item_starts_one_run_and_what_was_already_there_starts_none() {
    let rig = rig();
    let inputs = allow_runs(&rig);
    let job = poll_job(&rig, "j-poll", json!({ "query": "tasks", "every_minutes": 5, "state": "todo" }));
    let answer = Arc::new(Mutex::new(tasks(&[1, 2])));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let check = fake(&answer, &asked);
    let mut state = PollState::default();
    let all = [job.clone()];
    let t0 = agentmux_common::time::now_ms();

    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0).await, 0, "the first poll only takes note");
    assert_eq!(*asked.lock().unwrap(), [json!({ "query": "tasks", "state": "todo", "limit": 200 })], "it asks the query with its own filters and the most it gives, not how often");
    assert!(rig.runs("j-poll").is_empty());

    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + MIN).await, 0, "not due yet: nothing asked");
    assert_eq!(asked.lock().unwrap().len(), 1);

    *answer.lock().unwrap() = tasks(&[1, 2, 3]);
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 5 * MIN).await, 1);
    until_runs(&rig, "j-poll", 1).await;
    let run = rig.runs("j-poll").remove(0);
    assert_eq!((run.trigger.as_str(), run.triggered_by.as_str()), ("event", "event:poll"));
    assert_eq!(
        *inputs.lock().unwrap(),
        [json!({ "query": "tasks", "key": "3", "item": { "num": 3, "title": "task 3", "state": "todo" } })],
        "the run reads the item"
    );

    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 10 * MIN).await, 0, "the same answer is nothing new");
    *answer.lock().unwrap() = tasks(&[1, 2]);
    poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 15 * MIN).await;
    *answer.lock().unwrap() = tasks(&[1, 2, 3]);
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 20 * MIN).await, 0, "an item that leaves and comes back is not new: its id is remembered");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(rig.runs("j-poll").len(), 1);
}

async fn until_runs(rig: &Rig, job: &str, n: usize) {
    for _ in 0..100 {
        if rig.runs(job).len() >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("{n} runs of {job}");
}

/// A restart forgets the poller's memory, not what the job has heard: what was reported stays reported, and what
/// appeared while srv was off is new.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_what_was_reported_is_not_again_and_what_came_meanwhile_is() {
    let rig = rig();
    allow_runs(&rig);
    let job = poll_job(&rig, "j-restart", json!({ "query": "tasks", "every_minutes": 5 }));
    let answer = Arc::new(Mutex::new(tasks(&[1])));
    let check = fake(&answer, &Arc::default());
    let all = [job];
    let t0 = agentmux_common::time::now_ms();
    let mut before = PollState::default();
    poll::poll_due(&rig.sched, &all, &check, &mut before, t0).await;
    *answer.lock().unwrap() = tasks(&[1, 2]);
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut before, t0 + 5 * MIN).await, 1);

    *answer.lock().unwrap() = tasks(&[1, 2, 3]);
    let mut after = PollState::default();
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut after, t0 + 60 * MIN).await, 1, "only the task that came meanwhile");
    until_runs(&rig, "j-restart", 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn include_existing_runs_for_what_is_there_and_a_flood_is_taken_a_few_at_a_time() {
    let rig = rig();
    allow_runs(&rig);
    let job = poll_job(&rig, "j-all", json!({ "query": "tasks", "every_minutes": 1, "include_existing": true }));
    let many: Vec<u32> = (1..=25).collect();
    let answer = Arc::new(Mutex::new(tasks(&many)));
    let check = fake(&answer, &Arc::default());
    let all = [job];
    let t0 = agentmux_common::time::now_ms();
    let mut state = PollState::default();
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0).await, poll::MAX_NEW_PER_POLL);
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + MIN).await, 5, "the rest at the next poll");
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 2 * MIN).await, 0);
    until_runs(&rig, "j-all", 25).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_that_fails_is_skipped_and_an_edited_question_starts_from_what_is_there() {
    let rig = rig();
    allow_runs(&rig);
    let job = poll_job(&rig, "j-edit", json!({ "query": "tasks", "every_minutes": 1 }));
    let answer = Arc::new(Mutex::new(serde_json::Value::Null));
    let check = fake(&answer, &Arc::default());
    let t0 = agentmux_common::time::now_ms();
    let mut state = PollState::default();
    assert_eq!(poll::poll_due(&rig.sched, &[job.clone()], &check, &mut state, t0).await, 0, "a failed question is not an empty answer");
    *answer.lock().unwrap() = tasks(&[1]);
    assert_eq!(poll::poll_due(&rig.sched, &[job.clone()], &check, &mut state, t0 + MIN).await, 0, "the first answer is the one taken note of");

    let mut edited = job.clone();
    edited.trigger_event = json!({ "source": "poll", "filter": { "query": "tasks", "every_minutes": 1, "holder": "agent2" }, "max_per_hour": 1000 });
    assert_eq!(poll::poll_due(&rig.sched, &[edited.clone()], &check, &mut state, t0 + 2 * MIN).await, 0, "a new question takes note again");
    *answer.lock().unwrap() = tasks(&[1, 2]);
    assert_eq!(poll::poll_due(&rig.sched, &[edited.clone()], &check, &mut state, t0 + 3 * MIN).await, 1);

    let mut off = edited;
    off.enabled = false;
    *answer.lock().unwrap() = tasks(&[1, 2, 3]);
    assert_eq!(poll::poll_due(&rig.sched, &[off], &check, &mut state, t0 + 4 * MIN).await, 0, "a job that is off asks nothing");
}

#[test]
fn each_query_names_its_items_and_a_problem_is_new_when_the_agent_goes_down_another_way() {
    let agents = json!({ "problems": [{ "agent": "agent2", "state": "stopped" }, { "agent": "agent3", "state": "error" }, { "agent": "agent3", "state": "error" }, { "state": "x" }] });
    let ids: Vec<String> = poll::items_of_for_test("agents", &agents).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["agent2:stopped", "agent3:error"], "one per id; an item with no id is not one");
    let runs = json!({ "runs": [{ "id": "r1" }, { "id": "r2" }] });
    assert_eq!(poll::items_of_for_test("jobs", &runs).len(), 2);
    let peers = json!({ "peers": [{ "host": "narko" }] });
    assert_eq!(poll::items_of_for_test("lan_peers", &peers)[0].0, "narko");
    assert!(poll::items_of_for_test("tasks", &json!({ "count": 0 })).is_empty());
}

/// The first look is kept with the job's event ids, which a week's sweep forgets: each look moves it forward, so a
/// restart after a long stop is still not a first look.
#[tokio::test(flavor = "multi_thread")]
async fn the_first_look_is_remembered_past_the_weeks_sweep_of_event_ids() {
    use crate::backend::storage::job_events::KEEP_MS;
    let rig = rig();
    allow_runs(&rig);
    let job = poll_job(&rig, "j-marker", json!({ "query": "tasks", "every_minutes": 1 }));
    let marker = poll::marker_of_for_test(&job).unwrap();
    let answer = Arc::new(Mutex::new(tasks(&[1])));
    let check = fake(&answer, &Arc::default());
    let all = [job];
    let mut state = PollState::default();
    let t0 = agentmux_common::time::now_ms();
    poll::poll_due(&rig.sched, &all, &check, &mut state, t0).await;
    for day in [3, 6] {
        poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + day * 24 * 60 * MIN).await;
    }
    let sweep_at = t0 + 8 * 24 * 60 * MIN;
    rig.store.job_events_forget_before(sweep_at - KEEP_MS).unwrap();
    assert!(!rig.store.job_event_claim("j-marker", &marker, "", sweep_at).unwrap(), "the marker went through the sweep");
    assert_eq!(rig.store.job_events_since("j-marker", 0).unwrap(), 0, "and it is not a run the job admitted");
}

/// A poll of a site outside AgentMux: written out, https, asked at most every few minutes, with no key of the call's own.
#[test]
fn a_poll_of_api_writes_its_site_out_and_asks_it_at_most_every_few_minutes() {
    let ok = json!({ "query": "api", "every_minutes": 15, "url": "https://api.github.com/repos/o/r/actions/runs?per_page=20", "auth": { "vault": "api-key-github", "as": "bearer" }, "headers": { "Accept": "application/vnd.github+json" }, "items": "workflow_runs", "id": "id", "limit": 20 });
    assert!(poll::validate_filter(&filter(ok.clone())).is_ok());
    assert!(poll::validate_filter(&filter(json!({ "query": "api", "every_minutes": 5, "url": "https://example.org/list" }))).is_ok(), "no key is fine for a public list");
    let with = |k: &str, v: serde_json::Value| {
        let mut f = ok.clone();
        f[k] = v;
        poll::validate_filter(&filter(f)).unwrap_err()
    };
    assert!(with("every_minutes", json!(4)).contains("every 5 minutes"), "a poll of a site is not every minute");
    assert!(with("url", json!("http://api.github.com/x")).contains("https"));
    assert!(with("url", json!("https://{{trigger.host}}/x")).contains("writes its url out"));
    assert!(with("url", json!("https://u:p@api.github.com/x")).contains("no user name"));
    assert!(with("url", json!("api.github.com")).contains("not an address"));
    assert!(with("url", json!("")).contains("has url"));
    assert!(with("auth", json!("api-key-github")).contains("auth"));
    assert!(with("auth", json!({ "vault": "" })).contains("names its Vault entry"));
    assert!(with("auth", json!({ "vault": "{{trigger.entry}}" })).contains("names its Vault entry in vault, written out"));
    assert!(with("auth", json!({ "vault": "k", "extra": 1 })).contains("takes vault and as"));
    assert!(with("headers", json!({ "X-A": "{{trigger.x}}" })).contains("no {{reference}}"));
    assert!(with("headers", json!(["a"])).contains("object"));
    assert!(with("items", json!(3)).contains("dotted path"));
    // Only a GET of a list: there is no `method` or `body` to give, and no other query's key.
    let method = poll::validate_filter(&filter({ let mut f = ok.clone(); f["method"] = json!("POST"); f })).unwrap_err();
    assert!(method.contains("no filter key \"method\""), "{method}");
    assert!(poll::validate_filter(&filter({ let mut f = ok.clone(); f["body"] = json!("x"); f })).is_err());
    assert!(poll::validate_filter(&filter({ let mut f = ok.clone(); f["holder"] = json!("a"); f })).is_err());
    assert!(poll::validate_filter(&filter({ let mut f = ok.clone(); f["url"] = json!(null); f })).is_err());
    let no_url = poll::validate_filter(&filter(json!({ "query": "api", "every_minutes": 5 }))).unwrap_err();
    assert!(no_url.contains("has url"), "{no_url}");
}

/// The items of a site's list start one run each, once, with the item as the run reads it; the question carries the
/// drone the job runs, which is who a Vault credential is granted to, and not how often.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_item_in_a_sites_list_starts_one_run_and_the_question_names_the_jobs_drone() {
    let rig = rig();
    let inputs = allow_runs(&rig);
    let job = poll_job(&rig, "j-api", json!({ "query": "api", "every_minutes": 5, "url": "https://api.example.org/runs", "items": "runs" }));
    let list = |ids: &[u32]| json!({ "items": ids.iter().map(|i| json!({ "id": i.to_string(), "item": { "id": i, "conclusion": "failure" } })).collect::<Vec<_>>() });
    let answer = Arc::new(Mutex::new(list(&[1, 2])));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let check = fake(&answer, &asked);
    let mut state = PollState::default();
    let all = [job.clone()];
    let t0 = agentmux_common::time::now_ms();

    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0).await, 0, "the first poll takes note of what is there");
    assert_eq!(
        *asked.lock().unwrap(),
        [json!({ "query": "api", "url": "https://api.example.org/runs", "items": "runs", "limit": 100, "drone_id": "d-1" })],
        "the question: the site, the path and the drone, never how often"
    );
    *answer.lock().unwrap() = list(&[1, 2, 3]);
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 5 * MIN).await, 1);
    until_runs(&rig, "j-api", 1).await;
    assert_eq!(*inputs.lock().unwrap(), [json!({ "query": "api", "key": "3", "item": { "id": 3, "conclusion": "failure" } })], "the run reads the item itself");
    assert_eq!(poll::poll_due(&rig.sched, &all, &check, &mut state, t0 + 10 * MIN).await, 0, "the same list is nothing new");
}

/// A poll of a site with a credential belongs to a drone job: another kind of job has no drone to grant it to.
#[tokio::test(flavor = "multi_thread")]
async fn a_poll_of_api_for_a_job_that_is_not_a_drone_is_asked_with_no_drone() {
    let rig = rig();
    let mut job = poll_job(&rig, "j-api-msg", json!({ "query": "api", "every_minutes": 5, "url": "https://api.example.org/runs", "auth": { "vault": "api-key-x" } }));
    job.action_kind = "message".to_string();
    job.action = json!({ "kind": "message" });
    let answer = Arc::new(Mutex::new(json!({ "items": [] })));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let check = fake(&answer, &asked);
    let mut state = PollState::default();
    poll::poll_due(&rig.sched, &[job], &check, &mut state, agentmux_common::time::now_ms()).await;
    assert_eq!(asked.lock().unwrap()[0]["drone_id"], "", "no drone: the check refuses a credential for it");
}
