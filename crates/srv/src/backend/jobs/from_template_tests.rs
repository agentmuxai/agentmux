// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job made from a template, and the update diff
//! (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4, §8 "Templates").

use serde_json::{json, Value};

use super::*;
use crate::backend::job_templates::JobTemplates;
use crate::backend::jobs::definition::{Action, Check};

const ID: &str = "acme.drift";

fn manifest(version: &str, extra: Value) -> Value {
    let mut m = json!({
        "templateVersion": 1,
        "id": ID,
        "name": "Docs drift",
        "version": version,
        "kind": "job",
        "description": "Find docs whose claims no longer match the code.",
        "params": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "title": "Repository" },
                "focus": { "type": "string", "enum": ["docs", "security"], "default": "docs" },
                "token": { "type": "string", "format": "credential" }
            },
            "required": ["repo"]
        },
        "trigger": { "kind": "cron", "cron": "0 6 * * 1" },
        "action": { "kind": "message" },
        "policy": { "catch_up": "latest", "timeout_minutes": 30, "caps": { "run_usd": 1 } },
        "outputs": ["kanban_task"],
        "secrets": [{ "name": "REPO_TOKEN", "param": "token" }]
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        m[k] = v;
    }
    m
}

/// A service in a temp folder holding `ID` at `version`.
fn service_with(tmp: &std::path::Path, version: &str, brief: &str, extra: Value) -> JobTemplates {
    let svc = JobTemplates::new(tmp.join("job-templates"), &tmp.join("data"), &[]);
    write(&svc, version, brief, extra);
    svc
}

fn write(svc: &JobTemplates, version: &str, brief: &str, extra: Value) {
    let dir = svc.templates_dir.join(ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("template.json"), serde_json::to_vec_pretty(&manifest(version, extra)).unwrap()).unwrap();
    std::fs::write(dir.join("brief.md"), brief).unwrap();
}

fn approve(svc: &JobTemplates) -> String {
    let hash = svc.get(ID).unwrap().0.hash;
    svc.approve(ID, &hash, "").unwrap();
    hash
}

fn request(extra: Value) -> Value {
    let mut def = json!({
        "jobVersion": 1,
        "template": { "id": ID, "version": "1.0.0" },
        "params": { "repo": "acme/site", "token": "vault-entry-1" },
        "action": { "kind": "message", "to": "uid-a" },
        "owner": "uid-a",
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        def[k] = v;
    }
    def
}

fn parsed(def: &Value) -> JobDefinition {
    JobDefinition::parse(def, Check { fresh: true, now_ms: 1_000 }).unwrap()
}

#[test]
fn an_approved_template_makes_a_pinned_job_with_its_brief_outputs_and_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}} for {{focus}} drift.", json!({}));
    let hash = approve(&svc);
    let mut def = request(json!({}));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    assert_eq!(job.template, Some(TemplateRef { id: ID.into(), version: "1.0.0".into(), hash }));
    assert_eq!(job.name, "Docs drift", "the template's name when the request has none");
    assert_eq!(brief_of(&def), "Check acme/site for docs drift.", "rendered, with the default for focus");
    assert_eq!(job.params["focus"], "docs");
    assert_eq!(job.outputs, vec!["kanban_task".to_string()]);
    assert_eq!(job.secrets, vec![crate::backend::jobs::definition::SecretRef { name: "REPO_TOKEN".into(), vault: "vault-entry-1".into() }]);
    assert_eq!(job.policy.timeout_minutes, Some(30));
    assert_eq!(job.policy.caps.as_ref().and_then(|c| c.run_usd), Some(1.0));
    assert!(matches!(job.trigger, crate::backend::jobs::definition::Trigger::Cron { ref cron, .. } if cron == "0 6 * * 1"));
}

#[test]
fn the_request_wins_over_the_templates_defaults_and_the_version_defaults_to_the_installed_one() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    approve(&svc);
    let mut def = request(json!({
        "name": "Mine",
        "template": { "id": ID },
        "trigger": { "kind": "every", "seconds": 3600 },
        "policy": { "timeout_minutes": 5, "caps": { "day_usd": 3 } },
    }));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    assert_eq!((job.name.as_str(), job.template.unwrap().version.as_str()), ("Mine", "1.0.0"));
    assert!(matches!(job.trigger, crate::backend::jobs::definition::Trigger::Every { seconds: 3600 }));
    assert_eq!(job.policy.timeout_minutes, Some(5));
    let caps = job.policy.caps.unwrap();
    assert_eq!((caps.run_usd, caps.day_usd), (Some(1.0), Some(3.0)), "caps merge key by key");
}

#[test]
fn a_template_default_this_build_cant_run_yet_is_left_out() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({ "policy": { "timeout_minutes": 30, "max_turns": 200 } }));
    approve(&svc);
    let mut def = request(json!({}));
    apply_with(&mut def, None, &svc).unwrap();
    assert!(def["policy"].get("max_turns").is_none(), "{}", def["policy"]);
    assert_eq!(parsed(&def).policy.timeout_minutes, Some(30));
}

#[test]
fn a_task_template_carries_its_brief_as_the_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Look at {{repo}}.", json!({ "action": { "kind": "task" } }));
    approve(&svc);
    let mut def = request(json!({ "action": { "title": "Drift" } }));
    apply_with(&mut def, None, &svc).unwrap();
    assert_eq!(def["action"], json!({ "kind": "task", "title": "Drift", "payload": "Look at acme/site." }));
    assert!(matches!(parsed(&def).action, Action::Task { .. }));
}

#[test]
fn a_template_that_is_not_approved_or_changed_since_makes_no_job() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    let err = apply_with(&mut request(json!({})), None, &svc).unwrap_err();
    assert!(err.contains("isn't approved"), "{err}");

    let hash = approve(&svc);
    write(&svc, "1.0.0", "Check {{repo}} and run something else.", json!({}));
    let err = apply_with(&mut request(json!({})), None, &svc).unwrap_err();
    assert!(err.contains("changed since the operator approved"), "{err}");

    write(&svc, "1.0.0", "Check {{repo}}.", json!({}));
    let mut pinned = request(json!({ "template": { "id": ID, "version": "1.0.0", "hash": hash } }));
    assert!(apply_with(&mut pinned, None, &svc).is_ok(), "back to what was approved");
    let mut stale = request(json!({ "template": { "id": ID, "version": "1.0.0", "hash": "0000" } }));
    assert!(apply_with(&mut stale, None, &svc).unwrap_err().contains("changed since the job's template pin"));
    let mut elsewhere = request(json!({ "template": { "id": "acme.other", "version": "1.0.0" } }));
    assert!(apply_with(&mut elsewhere, None, &svc).unwrap_err().contains("no job template acme.other"));
    let mut old = request(json!({ "template": { "id": ID, "version": "0.9.0" } }));
    assert!(apply_with(&mut old, None, &svc).unwrap_err().contains("is at version 1.0.0"));
}

#[test]
fn parameters_are_checked_against_the_templates_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    approve(&svc);
    for (params, why) in [
        (json!({ "token": "t" }), "\"repo\" is required"),
        (json!({ "repo": "r", "focus": "weather" }), "\"focus\""),
        (json!({ "repo": "r", "extra": 1 }), "takes no parameter"),
        (json!({ "repo": "r", "token": "has space" }), "Vault entry reference"),
        (json!({ "repo": 7 }), "isn't a valid string"),
    ] {
        let err = apply_with(&mut request(json!({ "params": params })), None, &svc).unwrap_err();
        assert!(err.contains(why), "{why}: {err}");
    }
    // An optional credential left out is no secret of the job.
    let mut def = request(json!({ "params": { "repo": "r" } }));
    apply_with(&mut def, None, &svc).unwrap();
    assert!(parsed(&def).secrets.is_empty());
}

#[test]
fn a_definition_without_a_template_is_left_alone() {
    let mut def = json!({ "jobVersion": 1, "name": "x" });
    let before = def.clone();
    apply(&mut def, None).unwrap();
    assert_eq!(def, before);
}

#[test]
fn an_update_shows_what_accepting_it_changes_once_the_operator_approved_it() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}} for {{focus}} drift.", json!({}));
    approve(&svc);
    let mut def = request(json!({ "policy": { "timeout_minutes": 45 } }));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    assert_eq!(update_diff_with(&job, &svc).status, UpdateStatus::None);

    // Version 1.1.0 is installed but not approved yet.
    write(
        &svc,
        "1.1.0",
        "Check {{repo}} for {{focus}} drift, then list stale links.",
        json!({ "outputs": ["kanban_task", "report_file"], "policy": { "catch_up": "latest", "timeout_minutes": 90, "caps": { "run_usd": 2 } } }),
    );
    let waiting = update_diff_with(&job, &svc);
    assert_eq!((waiting.status.clone(), waiting.to_version.as_str()), (UpdateStatus::NeedsApproval, "1.1.0"));
    assert!(waiting.reason.unwrap().contains("hasn't been approved"));
    assert!(waiting.brief.is_none(), "nothing is shown as accepted before it is approved");

    let new_hash = approve(&svc);
    let diff = update_diff_with(&job, &svc);
    assert_eq!((diff.status.clone(), diff.from_version.as_str(), diff.to_version.as_str()), (UpdateStatus::Available, "1.0.0", "1.1.0"));
    assert_eq!(diff.to_hash, new_hash);
    let brief = diff.brief.unwrap();
    assert_eq!(brief.before, "Check acme/site for docs drift.");
    assert_eq!(brief.after, "Check acme/site for docs drift, then list stale links.");
    assert_eq!(diff.outputs, Some(ListChange { added: vec!["report_file".into()], removed: vec![] }));
    assert!(diff.secrets.is_none());
    let fields: Vec<(&str, &str, &str)> = diff.policy.iter().map(|c| (c.field.as_str(), c.before.as_str(), c.after.as_str())).collect();
    assert_eq!(fields, vec![("caps.run_usd", "1", "2"), ("timeout_minutes", "45", "90")]);

    // Accepting is an edit that names the new pin: it takes the new brief,
    // outputs and the policy the diff showed, and moves the pin.
    let mut edit = serde_json::to_value(&job).unwrap();
    edit["template"] = json!({ "id": ID, "version": "1.1.0", "hash": diff.to_hash });
    apply_with(&mut edit, job.template.as_ref(), &svc).unwrap();
    let accepted = parsed(&edit);
    assert_eq!(accepted.template.unwrap().version, "1.1.0");
    assert_eq!(accepted.outputs, vec!["kanban_task".to_string(), "report_file".to_string()]);
    assert_eq!((accepted.policy.timeout_minutes, accepted.policy.caps.unwrap().run_usd), (Some(90), Some(2.0)));
    assert_eq!(update_diff_with(&accepted_job(&edit), &svc).status, UpdateStatus::None);
}

fn accepted_job(edit: &Value) -> JobDefinition {
    JobDefinition::parse(edit, Check { fresh: false, now_ms: 1_000 }).unwrap()
}

#[test]
fn an_update_the_jobs_parameters_cant_take_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    approve(&svc);
    let mut def = request(json!({}));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    let mut params = manifest("1.1.0", json!({}))["params"].clone();
    params["properties"]["branch"] = json!({ "type": "string" });
    params["required"] = json!(["repo", "branch"]);
    write(&svc, "1.1.0", "Check {{repo}} on {{branch}}.", json!({ "params": params }));
    approve(&svc);
    let diff = update_diff_with(&job, &svc);
    assert_eq!(diff.status, UpdateStatus::Available);
    assert!(diff.params_error.unwrap().contains("\"branch\" is required"));
    assert!(diff.brief.is_none());
}

#[test]
fn a_template_that_is_gone_offers_no_update() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    approve(&svc);
    let mut def = request(json!({}));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    svc.uninstall(ID).unwrap();
    let diff = update_diff_with(&job, &svc);
    assert_eq!(diff.status, UpdateStatus::Unavailable);
    assert!(diff.reason.unwrap().contains("isn't installed"));
    let plain = JobDefinition { template: None, ..job };
    assert_eq!(update_diff_with(&plain, &svc).status, UpdateStatus::None);
}

#[test]
fn an_edit_that_leaves_the_template_alone_keeps_the_jobs_own_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service_with(tmp.path(), "1.0.0", "Check {{repo}}.", json!({}));
    approve(&svc);
    let mut def = request(json!({ "policy": { "timeout_minutes": 45 } }));
    apply_with(&mut def, None, &svc).unwrap();
    let job = parsed(&def);
    // New params, same pin: the brief is rendered again, the policy is not touched.
    let mut edit = serde_json::to_value(&job).unwrap();
    edit["params"] = json!({ "repo": "acme/other", "token": "vault-entry-1" });
    apply_with(&mut edit, job.template.as_ref(), &svc).unwrap();
    assert_eq!(brief_of(&edit), "Check acme/other.");
    assert_eq!(parsed(&edit).policy.timeout_minutes, Some(45));
}
