// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Jobs tab's editor RPCs: who may save a job, a job made from a
//! template, the dry run, and the update diff
//! (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3, §5.1, §5.4, §7, §8).

use serde_json::json;

use super::*;
use crate::backend::job_templates::JobTemplates;
use crate::backend::jobs::RunLedger;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::jobs::from_template::UpdateStatus;

fn operator() -> RpcContext {
    RpcContext::operator_for_tests()
}

fn agent() -> RpcContext {
    RpcContext { agent_id: "edit-owner".to_string(), ..Default::default() }
}

/// A state whose job store and run ledger are its test store, with an agent
/// on claude and one on codex (which reports no cost).
fn state() -> AppState {
    let mut state = crate::server::tests::test_state();
    crate::server::cron::use_test_store_for_jobs(&mut state);
    for (uid, slug, provider) in [("uid-edit-owner", "edit-owner", "claude"), ("uid-edit-codex", "edit-codex", "codex")] {
        let mut def = test_agent_def(uid, slug, provider, "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    let ledger = RunLedger::new(state.identity_store.clone(), Some(state.mstore.clone()), state.broker.clone());
    ledger.install_jobs(state.mstore.clone());
    state.cron_scheduler.install_runs(ledger);
    state
}

fn definition(extra: Value) -> Value {
    let mut def = json!({
        "jobVersion": 1,
        "name": "Weekly docs drift",
        "owner": "uid-edit-owner",
        "trigger": { "kind": "cron", "cron": "0 6 * * 1", "tz": "America/New_York" },
        "action": { "kind": "message", "to": "uid-edit-owner", "brief": "Check the docs." },
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        def[k] = v;
    }
    def
}

fn req(job_id: Option<&str>, definition: Value) -> JobsSaveRequest {
    JobsSaveRequest { job_id: job_id.map(str::to_string), definition, confirm_no_cost: None }
}

fn created(state: &AppState, def: Value) -> String {
    let saved = save(state, req(None, def), &operator()).unwrap();
    assert!(saved.ok, "{saved:?}");
    state.cron_scheduler.cancel_job(&saved.job_id);
    saved.job_id
}

/// An approved template `id` at `version`, in the process-wide service.
fn approved_template(id: &str, version: &str, brief: &str, extra: Value) -> String {
    let svc = JobTemplates::for_tests();
    let dir = svc.templates_dir.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let mut m = json!({
        "templateVersion": 1, "id": id, "name": "Drift template", "version": version, "kind": "job",
        "description": "Find docs whose claims no longer match the code.",
        "params": { "type": "object", "properties": { "repo": { "type": "string" } }, "required": ["repo"] },
        "trigger": { "kind": "cron", "cron": "0 6 * * 1" },
        "action": { "kind": "message" },
        "policy": { "timeout_minutes": 30 },
        "outputs": ["kanban_task"],
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        m[k] = v;
    }
    std::fs::write(dir.join("template.json"), m.to_string()).unwrap();
    std::fs::write(dir.join("brief.md"), brief).unwrap();
    let hash = svc.get(id).unwrap().0.hash;
    svc.approve(id, &hash, "").unwrap();
    hash
}

/// A definition made from template `id`: no name and no trigger, so the
/// template's apply.
fn from_template(id: &str, extra: Value) -> Value {
    let mut def = json!({
        "jobVersion": 1,
        "owner": "uid-edit-owner",
        "template": { "id": id, "version": "1.0.0" },
        "params": { "repo": "acme/site" },
        "action": { "kind": "message", "to": "uid-edit-owner" },
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        def[k] = v;
    }
    def
}

// ── jobs.save ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn only_the_operators_proven_window_saves_a_job() {
    let state = state();
    for ctx in [agent(), RpcContext::default()] {
        let err = save(&state, req(None, definition(json!({}))), &ctx).unwrap_err();
        assert!(err.starts_with("FORBIDDEN"), "{err}");
    }
    assert!(state.mstore.cron_list().unwrap().is_empty(), "nothing was created");
    // An agent connection can't edit one either.
    let id = created(&state, definition(json!({})));
    let err = save(&state, req(Some(&id), definition(json!({ "name": "Renamed" }))), &agent()).unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().name, "Weekly docs drift");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operator_creates_a_job_with_a_zone_a_spawn_action_and_caps_above_the_defaults() {
    let state = state();
    let id = created(&state, definition(json!({ "policy": { "caps": { "run_usd": 50, "day_usd": 200 } } })));
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!((job.tz.as_str(), job.expression.as_str(), job.created_by.as_str()), ("America/New_York", "0 6 * * 1", "operator"));
    let caps = job.policy.caps.unwrap();
    assert_eq!((caps.run_usd, caps.day_usd), (Some(50.0), Some(200.0)), "an agent's would be held to the defaults");

    let spawn = definition(json!({ "action": { "kind": "spawn", "leader": "uid-edit-owner", "brief": "Do the thing." } }));
    let id = created(&state, spawn);
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().action_kind, "spawn");

    let err = save(&state, req(None, definition(json!({ "trigger": { "kind": "cron", "cron": "*/5 * * * *" } }))), &operator()).unwrap_err();
    assert!(err.contains("at most every 15 minutes"), "the definition's rules still apply: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_on_a_provider_with_no_cost_waits_for_the_operators_confirmation() {
    let state = state();
    let codex = definition(json!({ "action": { "kind": "message", "to": "uid-edit-codex", "brief": "Check." } }));
    let asked = save(&state, req(None, codex.clone()), &operator()).unwrap();
    assert!(!asked.ok);
    assert_eq!((asked.needs_confirmation.as_deref(), asked.provider.as_deref()), (Some("no_cost"), Some("codex")));
    assert!(state.mstore.cron_list().unwrap().is_empty());
    let confirmed = save(&state, JobsSaveRequest { confirm_no_cost: Some(true), ..req(None, codex) }, &operator()).unwrap();
    assert!(confirmed.ok && confirmed.notes.iter().any(|n| n.contains("timeout_minutes")), "{confirmed:?}");
    state.cron_scheduler.cancel_job(&confirmed.job_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_changes_only_what_differs_and_clone_data_comes_from_get() {
    let state = state();
    // A job from before the 15-minute rule: every minute, to a task.
    let mut old = crate::backend::cron::test_job("jobs-old", "check in", "edit-owner", "uid-edit-owner", "uid-edit-owner");
    old.created_at = now_ms() / 1000;
    state.mstore.cron_create(&old).unwrap();
    state.cron_scheduler.cancel_job("jobs-old");

    let mut def = get(&state, "jobs-old").unwrap();
    assert_eq!(def["trigger"]["cron"], "* * * * *");
    def["name"] = json!("Renamed in the editor");
    def["policy"]["caps"] = json!({ "run_usd": 12 });
    let saved = save(&state, req(Some("jobs-old"), def.clone()), &operator()).unwrap();
    assert!(saved.ok, "kept as it was, the every-minute trigger isn't refused: {saved:?}");
    state.cron_scheduler.cancel_job("jobs-old");
    let job = state.mstore.cron_get("jobs-old").unwrap().unwrap();
    assert_eq!((job.name.as_str(), job.expression.as_str()), ("Renamed in the editor", "* * * * *"));
    assert_eq!(job.policy.caps.unwrap().run_usd, Some(12.0));

    // Setting the trigger does apply the rule.
    def["trigger"] = json!({ "kind": "cron", "cron": "*/10 * * * *", "tz": "UTC" });
    let err = save(&state, req(Some("jobs-old"), def), &operator()).unwrap_err();
    assert!(err.contains("at most every 15 minutes"), "{err}");
    assert!(get(&state, "nope").unwrap_err().contains("not found"));
    assert!(save(&state, req(Some("nope"), definition(json!({}))), &operator()).unwrap_err().contains("not found"));
}

// ── a job from a template ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_job_from_an_approved_template_records_its_pin_and_the_rendered_brief() {
    let state = state();
    let hash = approved_template("acme.editfrom", "1.0.0", "Check {{repo}} for drift.", json!({}));
    let id = created(&state, from_template("acme.editfrom", json!({})));
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    let pin = job.template.clone().unwrap();
    assert_eq!((pin.id.as_str(), pin.version.as_str(), pin.hash.as_str()), ("acme.editfrom", "1.0.0", hash.as_str()));
    assert_eq!(job.prompt, "Check acme/site for drift.");
    assert_eq!(job.name, "Drift template", "the template names a job that has no name");
    assert_eq!((job.expression.as_str(), job.policy.timeout_minutes), ("0 6 * * 1", Some(30)));
    assert_eq!(job.outputs, vec!["kanban_task".to_string()]);

    let err = save(&state, req(None, from_template("acme.editfrom", json!({ "params": {} }))), &operator()).unwrap_err();
    assert!(err.contains("\"repo\" is required"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_template_that_is_not_approved_makes_no_job() {
    let state = state();
    let svc = JobTemplates::for_tests();
    let dir = svc.templates_dir.join("acme.editunapproved");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("template.json"),
        json!({ "templateVersion": 1, "id": "acme.editunapproved", "name": "Not yet", "version": "1.0.0", "kind": "job",
                "description": "A test.", "action": { "kind": "message" } })
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("brief.md"), "Report in.").unwrap();
    let err = save(&state, req(None, from_template("acme.editunapproved", json!({}))), &operator()).unwrap_err();
    assert!(err.contains("isn't approved"), "{err}");
    assert!(state.mstore.cron_list().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_approved_version_is_offered_and_accepting_it_moves_the_pin() {
    let state = state();
    approved_template("acme.editupdate", "1.0.0", "Check {{repo}}.", json!({}));
    let id = created(&state, from_template("acme.editupdate", json!({})));
    let listed = super::super::jobs::list(&state).await.unwrap();
    let row = listed.jobs.iter().find(|j| j.id == id).unwrap();
    assert_eq!(row.template.as_ref().map(|t| (t.version.as_str(), t.update.is_none())), Some(("1.0.0", true)));
    assert_eq!(update_diff(&state, &id).unwrap().status, UpdateStatus::None);

    // 1.1.0 installed, not approved: the row says so, and there is nothing to accept yet.
    let svc = JobTemplates::for_tests();
    let dir = svc.templates_dir.join("acme.editupdate");
    let mut m: Value = serde_json::from_slice(&std::fs::read(dir.join("template.json")).unwrap()).unwrap();
    m["version"] = json!("1.1.0");
    m["outputs"] = json!(["kanban_task", "report_file"]);
    std::fs::write(dir.join("template.json"), m.to_string()).unwrap();
    std::fs::write(dir.join("brief.md"), "Check {{repo}} and the links.").unwrap();
    let listed = super::super::jobs::list(&state).await.unwrap();
    let update = listed.jobs.iter().find(|j| j.id == id).unwrap().template.as_ref().unwrap().update.clone().unwrap();
    assert_eq!((update.version.as_str(), update.approved), ("1.1.0", false));
    assert_eq!(update_diff(&state, &id).unwrap().status, UpdateStatus::NeedsApproval);
    let err = save(&state, req(Some(&id), json!({ "template": { "id": "acme.editupdate", "version": "1.1.0" } })), &operator()).unwrap_err();
    assert!(err.contains("approval again"), "{err}");

    let hash = svc.get("acme.editupdate").unwrap().0.hash;
    svc.approve("acme.editupdate", &hash, "").unwrap();
    let diff = update_diff(&state, &id).unwrap();
    assert_eq!((diff.status.clone(), diff.to_hash.as_str()), (UpdateStatus::Available, hash.as_str()));
    assert_eq!(diff.brief.as_ref().map(|b| b.after.as_str()), Some("Check acme/site and the links."));
    assert_eq!(diff.outputs.as_ref().map(|o| o.added.clone()), Some(vec!["report_file".to_string()]));

    // Renaming it, or aiming it at another agent, doesn't touch the template's brief, so it isn't refused for being behind.
    assert!(save(&state, req(Some(&id), json!({ "name": "Renamed" })), &operator()).unwrap().ok);
    let mut def = get(&state, &id).unwrap();
    def["action"]["to"] = json!("uid-edit-codex");
    assert!(save(&state, req(Some(&id), def), &operator()).unwrap().needs_confirmation.is_some(), "a codex target asks for the no-cost confirmation, not for an update");
    state.cron_scheduler.cancel_job(&id);

    // Editing the parameters while the job is behind is refused, naming the way out.
    let err = save(&state, req(Some(&id), json!({ "params": { "repo": "acme/other" } })), &operator()).unwrap_err();
    assert!(err.contains("accept the template's update first"), "{err}");

    let accept = json!({ "template": { "id": "acme.editupdate", "version": "1.1.0", "hash": diff.to_hash } });
    assert!(save(&state, req(Some(&id), accept), &operator()).unwrap().ok);
    state.cron_scheduler.cancel_job(&id);
    let job = state.mstore.cron_get(&id).unwrap().unwrap();
    assert_eq!(job.template.unwrap().version, "1.1.0");
    assert_eq!(job.prompt, "Check acme/site and the links.");
    assert_eq!(job.outputs, vec!["kanban_task".to_string(), "report_file".to_string()]);
    assert_eq!(update_diff(&state, &id).unwrap().status, UpdateStatus::None);
    assert!(super::super::jobs::list(&state).await.unwrap().jobs.iter().find(|j| j.id == id).unwrap().template.as_ref().unwrap().update.is_none());
}

// ── jobs.dryrun ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_shows_the_brief_five_fires_in_the_jobs_zone_the_caps_and_what_it_declares() {
    let state = state();
    approved_template("acme.editdry", "1.0.0", "Check {{repo}}.", json!({ "outputs": ["deploy"], "params": {
        "type": "object", "required": ["repo"],
        "properties": { "repo": { "type": "string" }, "token": { "type": "string", "format": "credential" } } },
        "secrets": [{ "name": "DEPLOY_TOKEN", "param": "token" }] }));
    let mut def = from_template("acme.editdry", json!({ "trigger": { "kind": "cron", "cron": "0 6 * * *", "tz": "America/New_York" } }));
    def["params"] = json!({ "repo": "acme/site", "token": "vault-1" });
    let out = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(def) }, &operator()).unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert_eq!((out.action_kind.as_str(), out.brief.as_str(), out.tz.as_str()), ("message", "Check acme/site.", "America/New_York"));
    assert_eq!(out.fires.len(), 5);
    assert!(out.fires.windows(2).all(|w| w[0].at_ms < w[1].at_ms));
    assert!(out.fires.iter().all(|f| f.local.contains("06:00") && (f.local.ends_with("EDT") || f.local.ends_with("EST"))), "{:?}", out.fires);
    let caps = out.caps.unwrap();
    assert_eq!((caps.run_usd, caps.day_usd, caps.account_day_usd, caps.clamped), (5.0, 20.0, 50.0, false));
    assert_eq!(out.outputs, vec!["deploy".to_string()]);
    assert_eq!((out.secrets[0].name.as_str(), out.secrets[0].vault.as_str()), ("DEPLOY_TOKEN", "vault-1"));
    assert_eq!(out.group.as_deref(), Some("deploy"), "a job that deploys waits its turn with the others");

    // Read-only: nothing was created, nothing ran.
    assert!(state.mstore.cron_list().unwrap().is_empty());
    let runs = state.cron_scheduler.runs().unwrap().store().job_runs_for_job("dry-run", 10, None).unwrap();
    assert!(runs.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_of_a_stored_job_and_of_a_bad_definition() {
    let state = state();
    let id = created(&state, definition(json!({ "policy": { "caps": { "run_usd": 12 } } })));
    let stored = dry_run(&state, JobsDryRunRequest { job_id: Some(id.clone()), definition: None }, &operator()).unwrap();
    assert!(stored.ok);
    assert_eq!((stored.fires.len(), stored.caps.as_ref().map(|c| c.run_usd)), (5, Some(12.0)));
    assert!(stored.fires[0].at_ms > now_ms());

    let bad = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(definition(json!({ "trigger": { "kind": "cron", "cron": "nope" } }))) }, &operator()).unwrap();
    assert!(!bad.ok && bad.error.unwrap().contains("invalid cron"));
    let none = dry_run(&state, JobsDryRunRequest { job_id: None, definition: None }, &operator()).unwrap();
    assert!(!none.ok);
    let manual = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(definition(json!({ "trigger": { "kind": "manual" } }))) }, &operator()).unwrap();
    assert!(manual.ok && manual.fires.is_empty() && manual.notes.is_empty(), "a manual job has no fires, and that's fine");
    let past = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(definition(json!({ "trigger": { "kind": "once", "at": 1000 } }))) }, &operator()).unwrap();
    assert!(!past.ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_dry_run_shows_its_higher_caps_held_to_the_defaults_and_flags_a_no_cost_agent() {
    let state = state();
    let def = definition(json!({ "policy": { "caps": { "run_usd": 50 } } }));
    let as_agent = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(def.clone()) }, &agent()).unwrap();
    let caps = as_agent.caps.unwrap();
    assert_eq!((caps.run_usd, caps.clamped), (5.0, true));
    assert!(as_agent.notes.iter().any(|n| n.contains("only the operator raises a cap")));
    let as_operator = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(def) }, &operator()).unwrap();
    assert_eq!((as_operator.caps.unwrap().run_usd, as_operator.notes.len()), (50.0, 0));

    let codex = definition(json!({ "action": { "kind": "message", "to": "uid-edit-codex", "brief": "Check." } }));
    let out = dry_run(&state, JobsDryRunRequest { job_id: None, definition: Some(codex) }, &operator()).unwrap();
    assert!(out.notes.iter().any(|n| n.contains("codex") && n.contains("60-minute timeout")), "{:?}", out.notes);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_never_writes_a_run() {
    let state = state();
    let id = created(&state, definition(json!({})));
    let before = state.cron_scheduler.runs().unwrap().store().job_runs_for_job(&id, 10, None).unwrap().len();
    dry_run(&state, JobsDryRunRequest { job_id: Some(id.clone()), definition: None }, &operator()).unwrap();
    let after = state.cron_scheduler.runs().unwrap().store().job_runs_for_job(&id, 10, None).unwrap();
    assert_eq!(after.len(), before);
}
