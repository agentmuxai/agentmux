// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The job definition: what it accepts, what it refuses, and the published
//! schema agreeing with it (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1, §6, §8).

use serde_json::json;

use super::*;

const NOW: i64 = 1_792_800_000_000;

fn create() -> Check {
    Check { fresh: true, now_ms: NOW }
}

/// The §5.1 example, less the template (templates come later).
fn example() -> Value {
    json!({
        "jobVersion": 1,
        "name": "Weekly docs drift",
        "owner": "uid-owner",
        "params": { "repo": "agentmux", "doc_globs": ["docs/**"] },
        "trigger": { "kind": "cron", "cron": "0 6 * * 1", "tz": "America/New_York" },
        "action": { "kind": "message", "to": "uid-target", "brief": "Check the docs." },
        "policy": {
            "overlap": "skip", "catch_up": "latest", "timeout_minutes": 60, "retries": 2, "max_turns": 200,
            "group": "deploy", "caps": { "run_usd": 5, "day_usd": 20, "messages_per_day": 24 },
            "approval": "none", "results": ["ledger", "task_on_failure"]
        },
        "outputs": ["report_file", "kanban_task", "message:owner", "pull_request"],
        "secrets": [{ "name": "SCANNER_TOKEN", "vault": "vault-entry-1" }],
        "enabled": true
    })
}

fn with(mut v: Value, pointer: &str, value: Value) -> Value {
    *v.pointer_mut(pointer).unwrap_or_else(|| panic!("{pointer}")) = value;
    v
}

/// `v` with `key` added to the object at `pointer`.
fn add(mut v: Value, pointer: &str, key: &str, value: Value) -> Value {
    v.pointer_mut(pointer).and_then(Value::as_object_mut).unwrap_or_else(|| panic!("{pointer}")).insert(key.into(), value);
    v
}

fn refused(v: Value) -> String {
    JobDefinition::parse(&v, create()).expect_err("refused")
}

#[test]
fn the_spec_example_reads_and_round_trips() {
    let def = JobDefinition::parse(&example(), create()).unwrap();
    assert_eq!(def.trigger, Trigger::Cron { cron: "0 6 * * 1".into(), tz: "America/New_York".into() });
    assert_eq!(def.policy.catch_up, CatchUp::Latest);
    assert_eq!(def.policy.caps.as_ref().unwrap().messages_per_day, Some(24));
    let again = JobDefinition::parse(&serde_json::to_value(&def).unwrap(), create()).unwrap();
    assert_eq!(again, def);
}

#[test]
fn defaults_fill_what_a_definition_leaves_out() {
    let def = JobDefinition::parse(
        &json!({ "jobVersion": 1, "name": "n", "trigger": { "kind": "cron", "cron": "0 6 * * 1" },
                 "action": { "kind": "message", "to": "a", "brief": "b" } }),
        create(),
    )
    .unwrap();
    assert_eq!(def.trigger, Trigger::Cron { cron: "0 6 * * 1".into(), tz: "UTC".into() });
    assert_eq!((def.policy.overlap, def.policy.catch_up, def.policy.retries), (Overlap::Skip, CatchUp::Skip, 0));
    assert_eq!(def.policy.approval, "none");
    assert_eq!(def.policy.results, ["ledger", "task_on_failure"]);
    assert!(def.enabled);
}

/// The kinds reserved for later phases validate as kinds and are refused
/// with "available in a later version" (§6).
#[test]
fn event_triggers_drone_actions_and_approvals_are_for_later() {
    let e = refused(with(example(), "/trigger", json!({ "kind": "event", "source": "github", "filter": {} })));
    assert_eq!(e, "trigger kind event is available in a later version");
    let e = refused(with(example(), "/action", json!({ "kind": "drone", "drone_id": "d-1" })));
    assert_eq!(e, "action kind drone is available in a later version");
    let e = refused(with(example(), "/policy/approval", json!("operator")));
    assert!(e.contains("policy.approval") && e.contains(LATER), "{e}");
}

/// The definition reads a template pin; whether the template exists and is
/// approved is checked where the job is made (`from_template`).
#[test]
fn a_template_pin_needs_an_id_and_a_version() {
    let pinned = add(example(), "", "template", json!({ "id": "agentmux.docs-drift", "version": "1.0.0", "hash": "x" }));
    assert!(JobDefinition::parse(&pinned, create()).is_ok());
    let unversioned = add(example(), "", "template", json!({ "id": "agentmux.docs-drift", "version": " " }));
    assert_eq!(refused(unversioned), "template: id and version are required");
}

/// An unattended agent job runs at most every 15 minutes, checked when its
/// trigger or action is set; a task job isn't an agent run.
#[test]
fn an_agent_job_runs_at_most_every_fifteen_minutes() {
    let every_5 = with(example(), "/trigger", json!({ "kind": "cron", "cron": "*/5 * * * *" }));
    assert!(refused(every_5.clone()).contains("at most every 15 minutes"), "{}", refused(every_5.clone()));
    let pair = with(example(), "/trigger", json!({ "kind": "cron", "cron": "0,10 6 * * *" }));
    assert!(refused(pair).contains("10 min apart"));
    let every_10m = with(example(), "/trigger", json!({ "kind": "every", "seconds": 600 }));
    assert!(refused(every_10m).contains("15 minutes"));
    assert!(JobDefinition::parse(&with(example(), "/trigger", json!({ "kind": "cron", "cron": "*/15 * * * *" })), create()).is_ok());

    // Kept as it was (an edit that leaves trigger and action alone).
    assert!(JobDefinition::parse(&every_5, Check { fresh: false, now_ms: NOW }).is_ok());
    // Filing a task starts no agent.
    let task = with(every_5, "/action", json!({ "kind": "task", "title": "triage" }));
    assert!(JobDefinition::parse(&task, create()).is_ok());
}

#[test]
fn trigger_values_are_checked() {
    assert!(refused(with(example(), "/trigger/tz", json!("Mars/Olympus"))).contains("unknown time zone"));
    assert!(refused(with(example(), "/trigger/cron", json!("0 6 * *"))).contains("invalid cron"));
    assert!(refused(with(example(), "/trigger", json!({ "kind": "every", "seconds": 30 }))).contains("once a minute"));
    assert!(refused(with(example(), "/trigger", json!({ "kind": "once", "at": NOW - 1 }))).contains("ahead"));
    assert!(JobDefinition::parse(&with(example(), "/trigger", json!({ "kind": "once", "at": NOW + 1 })), create()).is_ok());
    assert!(JobDefinition::parse(&with(example(), "/trigger", json!({ "kind": "manual" })), create()).is_ok());
    assert!(refused(with(example(), "/trigger", json!({ "kind": "hourly" }))).contains("invalid job definition"));
}

#[test]
fn unknown_fields_are_refused_where_they_are() {
    assert_eq!(refused(add(example(), "", "schedule", json!("daily"))), "unknown field schedule");
    assert_eq!(refused(add(example(), "/policy", "overlpa", json!("skip"))), "unknown field policy.overlpa");
    assert_eq!(refused(add(example(), "/trigger", "every", json!(5))), "unknown field trigger.every");
    assert_eq!(refused(add(example(), "/action", "model", json!("x"))), "unknown field action.model");
    assert_eq!(refused(add(example(), "/policy/caps", "hour_usd", json!(1))), "unknown field policy.caps.hour_usd");
}

#[test]
fn policy_values_are_checked() {
    assert_eq!(refused(with(example(), "/policy/retries", json!(4))), "policy.retries: at most 3");
    assert!(refused(with(example(), "/policy/overlap", json!("queue"))).contains("invalid job definition"));
    assert!(refused(with(example(), "/policy/timeout_minutes", json!(0))).contains("timeout_minutes"));
    assert!(refused(with(example(), "/policy/group", json!("Deploy!"))).contains("policy.group"));
    assert!(refused(with(example(), "/policy/caps/run_usd", json!(-1))).contains("policy.caps"));
    assert!(refused(with(example(), "/policy/results", json!(["email"]))).contains("policy.results"));
    for overlap in ["skip", "buffer_one", "cancel_other"] {
        assert!(JobDefinition::parse(&with(example(), "/policy/overlap", json!(overlap)), create()).is_ok(), "{overlap}");
    }
}

#[test]
fn outputs_secrets_and_the_rest_are_checked() {
    assert!(refused(with(example(), "/outputs", json!(["Report File"]))).contains("outputs"));
    assert!(refused(with(example(), "/secrets/0/name", json!("AGENTMUX_AUTH_KEY"))).contains("AGENTMUX_"));
    assert!(refused(with(example(), "/secrets/0/vault", json!(" "))).contains("Vault entry"));
    let twice = json!([{ "name": "A", "vault": "1" }, { "name": "A", "vault": "2" }]);
    assert!(refused(with(example(), "/secrets", twice)).contains("twice"));
    assert!(refused(with(example(), "/jobVersion", json!(2))).contains("jobVersion 2"));
    assert!(refused(with(example(), "/name", json!(" "))).contains("name"));
    assert!(refused(with(example(), "/action/brief", json!(""))).contains("message needs"));
    assert!(refused(with(example(), "/action", json!({ "kind": "spawn", "leader": "", "brief": "b" }))).contains("spawn needs"));
}

/// A job written from a definition reads back as the same definition.
#[test]
fn a_definition_written_into_a_job_reads_back_the_same() {
    let mut def = JobDefinition::parse(&example(), create()).unwrap();
    def.id = "job-1".into();
    def.created_by = "uid-creator".into();
    let mut job = CronJob { id: "job-1".into(), created_by_uid: "uid-creator".into(), ..Default::default() };
    def.apply_to(&mut job, &|to| ("target-name".to_string(), to.to_string()));
    assert_eq!((job.expression.as_str(), job.tz.as_str(), job.catch_up.as_str()), ("0 6 * * 1", "America/New_York", "latest"));
    assert_eq!((job.target.as_str(), job.target_uid.as_str(), job.prompt.as_str()), ("target-name", "uid-target", "Check the docs."));
    assert_eq!(JobDefinition::from_job(&job), def);

    let spawn = with(example(), "/action", json!({ "kind": "spawn", "leader": "uid-lead", "brief": "go", "keep_worker": true }));
    let mut def = JobDefinition::parse(&spawn, create()).unwrap();
    (def.id, def.created_by) = ("job-1".into(), "uid-creator".into());
    def.apply_to(&mut job, &|_| unreachable!("a spawn has no message target"));
    assert_eq!((job.action_kind.as_str(), job.target.as_str()), ("spawn", ""));
    assert_eq!(JobDefinition::from_job(&job), def);
}

/// The published schema names the same kinds and values the code reads.
#[test]
fn the_published_schema_matches_the_definition() {
    let schema: Value = serde_json::from_str(include_str!("../../../../../schema/agentmux-job.v1.schema.json")).unwrap();
    let enum_of = |pointer: &str| -> Vec<String> {
        let mut v: Vec<String> = schema
            .pointer(pointer)
            .unwrap_or_else(|| panic!("{pointer}"))
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    let sorted = |xs: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = xs.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(schema["properties"]["jobVersion"]["const"], JOB_VERSION);
    assert_eq!(enum_of("/$defs/trigger/properties/kind/enum"), sorted(&trigger_kind::ALL));
    assert_eq!(enum_of("/$defs/action/properties/kind/enum"), sorted(&action_kind::ALL));
    assert_eq!(enum_of("/$defs/policy/properties/overlap/enum"), sorted(&["skip", "buffer_one", "cancel_other"]));
    assert_eq!(enum_of("/$defs/policy/properties/catch_up/enum"), sorted(&["skip", "latest"]));
    assert_eq!(enum_of("/$defs/policy/properties/approval/enum"), sorted(&["none"]));
    assert_eq!(enum_of("/$defs/policy/properties/results/items/enum"), sorted(&RESULTS));
    let keys = |pointer: &str| -> Vec<String> {
        let mut v: Vec<String> = schema.pointer(pointer).unwrap().as_object().unwrap().keys().cloned().collect();
        v.sort();
        v
    };
    assert_eq!(keys("/properties"), sorted(TOP_KEYS));
    assert_eq!(keys("/$defs/policy/properties"), sorted(POLICY_KEYS));
    assert_eq!(keys("/$defs/caps/properties"), sorted(CAPS_KEYS));
    assert_eq!(schema["$defs"]["policy"]["properties"]["retries"]["maximum"], 3);
}
