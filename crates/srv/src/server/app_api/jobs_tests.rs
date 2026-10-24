// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Jobs tab's RPCs: the list, a job's runs, the actions, Pause all, and
//! who may call them (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3, §7).

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::backend::cron::test_job;
use crate::backend::storage::job_runs::{infra, state as run_state};

/// A state whose cron store and run ledger are its test store, with fires
/// answered as delivered to `b-jobs` and counted in the returned list.
fn state() -> (AppState, Arc<Mutex<Vec<String>>>) {
    let mut state = crate::server::tests::test_state();
    state.shared_store = Some(state.mstore.clone());
    let ledger = RunLedger::new(state.identity_store.clone(), None, state.broker.clone());
    state.cron_scheduler.install_runs(ledger);
    let fired: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = fired.clone();
    state.cron_scheduler.install_delivery(Arc::new(move |req, _uid| {
        sink.lock().unwrap().push(req.target_agent.clone());
        Box::pin(async move { json!({"success": true, "block_id": "b-jobs"}) })
    }));
    (state, fired)
}

fn job(state: &AppState, id: &str, creator_uid: &str) -> CronJob {
    let mut job = test_job(id, "check in", "jobs-target", "uid-jobs-target", creator_uid);
    job.created_at = now_ms() / 1000;
    state.mstore.cron_create(&job).unwrap();
    job
}

fn operator() -> RpcContext {
    RpcContext::default()
}

fn agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.to_string(), ..Default::default() }
}

fn act_req(action: &str, job_id: Option<&str>) -> JobsActRequest {
    JobsActRequest { action: action.to_string(), job_id: job_id.map(str::to_string) }
}

fn view<'a>(list: &'a JobsListResult, id: &str) -> &'a JobView {
    list.jobs.iter().find(|j| j.id == id).expect("listed")
}

fn ledger_of(state: &AppState) -> Arc<RunLedger> {
    state.cron_scheduler.runs().cloned().unwrap()
}

/// A finished run of `job_id`, `infra_status`, started `ago_ms` before now.
fn finished(state: &AppState, job_id: &str, infra_status: &str, cost: f64, ago_ms: i64) {
    let mut run = JobRun::new(&uuid::Uuid::new_v4().to_string(), job_id, "cron", "n", "schedule", run_state::FINISHED, now_ms() - ago_ms);
    run.infra = infra_status.to_string();
    run.cost_usd = cost;
    run.cost_known = true;
    ledger_of(state).store().job_run_insert(&run).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_shows_each_job_with_its_runs_spend_and_whether_it_is_failing() {
    let (state, _) = state();
    job(&state, "jobs-a", "uid-a");
    job(&state, "jobs-b", "uid-a");
    finished(&state, "jobs-a", infra::OK, 0.25, 2 * DAY_MS);
    finished(&state, "jobs-a", infra::ERROR, 0.5, 1_000);
    finished(&state, "jobs-b", infra::OK, 0.0, 1_000);

    let list = list(&state).await.unwrap();
    let a = view(&list, "jobs-a");
    assert_eq!((a.source.as_str(), a.state.as_str(), a.actions), ("cron", "active", true));
    assert!(a.next_run_ms.is_some_and(|n| n > list.now_ms), "an enabled job has a next run");
    assert_eq!(a.recent.len(), 2);
    assert!(a.recent[0].failed && !a.recent[1].failed, "newest first, each with its verdict");
    assert!(a.failing);
    assert!((a.cost_week_usd - 0.75).abs() < 1e-9, "{}", a.cost_week_usd);
    assert!((a.cost_today_usd - 0.5).abs() < 1e-9, "today counts only the newest run: {}", a.cost_today_usd);
    assert!(!view(&list, "jobs-b").failing);
    assert!(list.paused_all.is_none());
    // The run JSON is flat: the ledger row plus `failed`.
    let row = serde_json::to_value(&a.recent[0]).unwrap();
    assert_eq!((row["infra"].as_str(), row["failed"].as_bool()), (Some("error"), Some(true)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_stays_failing_behind_more_skips_than_the_strip_shows_until_a_success() {
    let (state, _) = state();
    job(&state, "jobs-f", "uid-a");
    finished(&state, "jobs-f", infra::ERROR, 0.0, 100_000);
    let store = ledger_of(&state).store().clone();
    for i in 0..20 {
        let started = now_ms() - 90_000 + i * 1_000;
        let mut skip = JobRun::new(&format!("skip-{i}"), "jobs-f", "cron", "n", "schedule", run_state::SKIPPED, started);
        skip.reason = "paused".into();
        store.job_run_insert(&skip).unwrap();
    }
    let list_now = list(&state).await.unwrap();
    let f = view(&list_now, "jobs-f");
    assert_eq!(f.recent.len(), 14);
    assert!(f.recent.iter().all(|r| r.run.state == "skipped"), "the failure is off the strip");
    assert!(f.failing, "still failing behind 20 skips");

    finished(&state, "jobs-f", infra::OK, 0.0, 0);
    assert!(!view(&list(&state).await.unwrap(), "jobs-f").failing, "a success ends it");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_page_newest_first() {
    let (state, _) = state();
    job(&state, "jobs-p", "uid-a");
    for ago in [3_000, 2_000, 1_000] {
        finished(&state, "jobs-p", infra::OK, 0.0, ago);
    }
    let first = runs(&state, JobsRunsRequest { job_id: "jobs-p".into(), limit: Some(2), before: None }).await.unwrap();
    assert_eq!(first.runs.len(), 2);
    let before = first.runs[1].run.started_at_ms;
    let rest = runs(&state, JobsRunsRequest { job_id: "jobs-p".into(), limit: Some(2), before: Some(before) }).await.unwrap();
    assert_eq!(rest.runs.len(), 1);
    assert!(rest.runs[0].run.started_at_ms < before);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operator_pauses_resumes_runs_and_deletes_a_job() {
    let (state, fired) = state();
    job(&state, "jobs-act", "uid-a");

    act(&state, act_req("pause", Some("jobs-act")), &operator()).await.unwrap();
    assert_eq!(view(&list(&state).await.unwrap(), "jobs-act").state, "paused");
    act(&state, act_req("resume", Some("jobs-act")), &operator()).await.unwrap();
    assert_eq!(view(&list(&state).await.unwrap(), "jobs-act").state, "active");
    state.cron_scheduler.cancel_job("jobs-act");

    let ran = act(&state, act_req("run_now", Some("jobs-act")), &operator()).await.unwrap();
    assert_eq!((ran.ok, ran.outcome.as_deref()), (true, Some("delivered")));
    assert_eq!(fired.lock().unwrap().as_slice(), ["uid-jobs-target"]);
    let run = ledger_of(&state).store().job_run_get(ran.run_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((run.trigger.as_str(), run.triggered_by.as_str()), ("manual", "operator"));

    act(&state, act_req("delete", Some("jobs-act")), &operator()).await.unwrap();
    assert!(list(&state).await.unwrap().jobs.iter().all(|j| j.id != "jobs-act"));
    assert!(act(&state, act_req("pause", Some("jobs-act")), &operator()).await.unwrap_err().contains("not found"));
    assert!(act(&state, act_req("pause", None), &operator()).await.is_err(), "a job is required");
    assert!(act(&state, act_req("explode", Some("jobs-act")), &operator()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_connection_acts_only_on_its_own_jobs_and_never_pauses_all() {
    let (state, _) = state();
    let slug = format!("jobs-agent-{}", uuid::Uuid::new_v4().simple());
    let uid = format!("uid-{slug}");
    state
        .reactive_handler
        .register_agent_full(&slug, &format!("b-{slug}"), None, 0, None, Some(&uid), "test.jobs")
        .unwrap();
    job(&state, "jobs-mine", &uid);
    job(&state, "jobs-theirs", "uid-someone-else");

    act(&state, act_req("pause", Some("jobs-mine")), &agent(&slug)).await.unwrap();
    let refused = act(&state, act_req("pause", Some("jobs-theirs")), &agent(&slug)).await.unwrap_err();
    assert!(refused.contains("only a scheduled job it created"), "{refused}");
    assert!(state.mstore.cron_get("jobs-theirs").unwrap().unwrap().enabled);

    for action in ["pause_all", "resume_all"] {
        let err = act(&state, act_req(action, None), &agent(&slug)).await.unwrap_err();
        assert!(err.starts_with("FORBIDDEN"), "{err}");
    }
    assert!(ledger_of(&state).paused_all().is_none());
    // An agent connection with no identity here can't act at all.
    let err = act(&state, act_req("pause", Some("jobs-mine")), &agent("nobody-registered-here")).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_all_skips_every_fire_refuses_run_now_and_only_the_operator_lifts_it() {
    let (state, fired) = state();
    let j = job(&state, "jobs-all", "uid-a");

    act(&state, act_req("pause_all", None), &operator()).await.unwrap();
    let list_now = list(&state).await.unwrap();
    assert_eq!(list_now.paused_all.as_ref().map(|p| p.set_by.as_str()), Some("operator"));
    assert_eq!(view(&list_now, "jobs-all").state, "paused_all");

    let ran = act(&state, act_req("run_now", Some("jobs-all")), &operator()).await.unwrap();
    assert!(!ran.ok);
    assert!(ran.error.as_deref().unwrap_or("").contains("Pause all"), "{ran:?}");

    // Two scheduled slots: no delivery, one collapsed skip row.
    for slot in [60_000, 120_000] {
        let f = state.cron_scheduler.fire(&j, crate::backend::cron::Slot::Scheduled(slot)).await;
        assert!(f.outcome.is_none() && f.run_id.is_none());
    }
    assert!(fired.lock().unwrap().is_empty(), "nothing was delivered");
    let rows = ledger_of(&state).store().job_runs_for_job("jobs-all", 10, None).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].state.as_str(), rows[0].reason.as_str(), rows[0].skip_count), ("skipped", "paused_all", 2));
    let stored = state.mstore.cron_get("jobs-all").unwrap().unwrap();
    assert_eq!((stored.fire_count, stored.missed_count), (0, 0), "a skip the operator caused is neither");

    act(&state, act_req("resume_all", None), &operator()).await.unwrap();
    assert!(list(&state).await.unwrap().paused_all.is_none());
    let ran = act(&state, act_req("run_now", Some("jobs-all")), &operator()).await.unwrap();
    assert!(ran.ok, "{ran:?}");
}

#[test]
fn jobs_sort_by_next_run_then_name() {
    let mk = |name: &str, next: Option<i64>| JobView {
        id: name.into(),
        source: "cron".into(),
        name: name.into(),
        owner: String::new(),
        owner_uid: String::new(),
        owner_block_id: None,
        target: String::new(),
        expression: String::new(),
        next_run_ms: next,
        state: "active".into(),
        catch_up: String::new(),
        fire_count: 0,
        max_fires: None,
        missed_count: 0,
        recent: vec![],
        failing: false,
        cost_today_usd: 0.0,
        cost_week_usd: 0.0,
        cost_known: false,
        actions: true,
    };
    let mut jobs = vec![mk("b", None), mk("c", Some(5)), mk("a", None), mk("d", Some(1))];
    sort_jobs(&mut jobs);
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["d", "c", "a", "b"]);
}
