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
    crate::server::cron::use_test_store_for_jobs(&mut state);
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
    RpcContext::operator_for_tests()
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
    // Nor can a connection the host never proved is the AgentMux window's.
    for action in ["pause", "pause_all"] {
        let err = act(&state, act_req(action, Some("jobs-theirs")), &RpcContext::default()).await.unwrap_err();
        assert!(err.starts_with("FORBIDDEN") && err.contains("isn't the AgentMux window"), "{action}: {err}");
    }
    assert!(state.mstore.cron_get("jobs-theirs").unwrap().unwrap().enabled);
    assert!(ledger_of(&state).paused_all().is_none());
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
        trigger_kind: "cron".into(),
        tz: "UTC".into(),
        every_secs: None,
        once_at_ms: None,
        next_run_ms: next,
        state: "active".into(),
        catch_up: String::new(),
        fire_count: 0,
        max_fires: None,
        missed_count: 0,
        recent: vec![],
        failing: false,
        incident: None,
        cost_today_usd: 0.0,
        cost_week_usd: 0.0,
        cost_known: false,
        actions: true,
        over_cap: None,
        run_cap_usd: None,
        day_cap_usd: None,
        cost_not_reported: false,
        template: None,
        cloud: None,
    };
    let mut jobs = vec![mk("b", None), mk("c", Some(5)), mk("a", None), mk("d", Some(1))];
    sort_jobs(&mut jobs);
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["d", "c", "a", "b"]);
}

/// An approved cloud template is a job in the list: its schedule, reporter and runs, and the events dropped.
#[tokio::test(flavor = "multi_thread")]
async fn an_approved_cloud_job_is_listed_with_its_reporter_and_its_missed_slots() {
    use crate::backend::job_templates::JobTemplates;
    use crate::backend::widget_signature::test_keys;
    let (state, _) = state();
    let (id, key) = ("acme.cloudlist", test_keys::public_key(31));
    let svc = JobTemplates::for_tests();
    let dir = svc.templates_dir.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("template.json"),
        json!({ "templateVersion": 1, "id": id, "name": "Cloud nightly", "version": "1.0.0", "kind": "cloud", "description": "A test.",
                "owner": "uid-cloud-owner", "reporter": { "name": "list-cloud", "key": key },
                "expect": { "cron": "0 2 * * *", "tz": "UTC", "graceMinutes": 30 }, "maxDurationMinutes": 120,
                "runNow": { "link": "https://ci.example/run" } })
        .to_string(),
    )
    .unwrap();
    let before = list(&state).await.unwrap();
    assert!(before.jobs.iter().all(|j| j.id != id), "an unapproved template is not a job");
    let hash = svc.get(id).unwrap().0.hash;
    svc.approve(id, &hash, "").unwrap();

    let mut missed = JobRun::new(&format!("watchdog:{id}:1"), id, "cloud", "Cloud nightly", "schedule", run_state::MISSED, now_ms() - 1_000);
    missed.scheduled_for_ms = Some(now_ms() - 1_000);
    ledger_of(&state).store().job_run_insert(&missed).unwrap();

    let listed = list(&state).await.unwrap();
    let cloud = view(&listed, id);
    assert_eq!((cloud.source.as_str(), cloud.name.as_str(), cloud.trigger_kind.as_str()), ("cloud", "Cloud nightly", "cron"));
    assert_eq!((cloud.expression.as_str(), cloud.tz.as_str(), cloud.actions), ("0 2 * * *", "UTC", false));
    assert!(cloud.next_run_ms.is_some_and(|n| n > now_ms()));
    assert_eq!(cloud.recent.len(), 1);
    assert!(cloud.failing, "a missed slot is a failure");
    let c = cloud.cloud.as_ref().expect("cloud fields");
    assert_eq!((c.reporter.as_str(), c.reporter_pinned, c.grace_minutes, c.max_run_minutes), ("list-cloud", true, Some(30), Some(120)));
    assert_eq!(c.run_now_link.as_deref(), Some("https://ci.example/run"));
    // The owner is a uid in this manifest and nobody here has it: it stays as written, with no pane to open.
    assert_eq!((cloud.owner.as_str(), cloud.owner_uid.as_str(), cloud.owner_block_id.as_deref()), ("uid-cloud-owner", "uid-cloud-owner", None));
    // The reporter forgotten: the job stays, and says its events would be dropped.
    svc.reporters().forget("list-cloud").unwrap();
    let after = list(&state).await.unwrap();
    assert!(!view(&after, id).cloud.as_ref().unwrap().reporter_pinned);
}

/// A manifest names its owner by agent name or by uid (§4.3): either finds the agent's pane here.
#[tokio::test(flavor = "multi_thread")]
async fn a_cloud_job_finds_its_owner_by_name_or_by_uid() {
    use crate::backend::job_templates::JobTemplates;
    use crate::backend::widget_signature::test_keys;
    let (state, _) = state();
    state.reactive_handler.register_agent("Cloudy-Owner", "b-cloudy", None).unwrap();
    let uid = state.reactive_handler.list_agents().into_iter().find(|r| r.agent_id == "Cloudy-Owner").and_then(|r| r.uid);
    let svc = JobTemplates::for_tests();
    for (id, seed, owner) in [("acme.cloudownername", 32u8, "cloudy-owner".to_string()), ("acme.cloudowneruid", 33u8, uid.clone().unwrap_or_else(|| "no-uid".into()))] {
        let dir = svc.templates_dir.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("template.json"),
            json!({ "templateVersion": 1, "id": id, "name": "Owned", "version": "1.0.0", "kind": "cloud", "description": "A test.",
                    "owner": owner, "reporter": { "name": format!("owner-rep-{seed}"), "key": test_keys::public_key(seed) },
                    "expect": "none", "maxDurationMinutes": 60 })
            .to_string(),
        )
        .unwrap();
        let hash = svc.get(id).unwrap().0.hash;
        svc.approve(id, &hash, "").unwrap();
    }
    let listed = list(&state).await.unwrap();
    let by_name = view(&listed, "acme.cloudownername");
    assert_eq!((by_name.owner.as_str(), by_name.owner_block_id.as_deref()), ("Cloudy-Owner", Some("b-cloudy")));
    assert_eq!(by_name.owner_uid.as_str(), uid.as_deref().unwrap_or("cloudy-owner"));
    if uid.is_some() {
        let by_uid = view(&listed, "acme.cloudowneruid");
        assert_eq!((by_uid.owner.as_str(), by_uid.owner_block_id.as_deref()), ("Cloudy-Owner", Some("b-cloudy")));
    }
}
