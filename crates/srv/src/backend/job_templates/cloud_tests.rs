// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of cloud job manifests (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.3, §8).

use serde_json::{json, Value};

use super::cloud::CloudExpect;
use super::tests::{manifest_json, write_template, BRIEF};
use super::*;
use crate::backend::jobs::cloud_events::{CloudJob, CloudJobs};
use crate::backend::widget_signature::test_keys;

const ID: &str = "acme.nightly";

/// A cloud manifest for `id`, its reporter the key of seed 1, with `extra` laid over it (`null` removes a field).
fn cloud(id: &str, extra: Value) -> Value {
    let mut m = manifest_json(
        id,
        json!({
            "kind": "cloud", "name": "Nightly", "params": null, "trigger": null, "action": null, "policy": null, "outputs": null,
            "owner": "ops",
            "reporter": { "name": "acme-ci", "key": test_keys::public_key(1) },
            "expect": { "cron": "0 2 * * *", "tz": "UTC", "graceMinutes": 30 },
            "maxDurationMinutes": 120,
            "runNow": { "link": "https://ci.example.com/run/nightly" },
        }),
    );
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        if v.is_null() {
            m.as_object_mut().unwrap().remove(&k);
        } else {
            m[k] = v;
        }
    }
    m
}

fn parse(m: &Value) -> Result<package::Package, String> {
    let id = m["id"].as_str().unwrap().to_string();
    let files = [("template.json".to_string(), serde_json::to_vec(m).unwrap())];
    package::parse(&id, &files.into_iter().collect())
}

fn service(tmp: &std::path::Path) -> JobTemplates {
    JobTemplates::new(tmp.join("job-templates"), &tmp.join("data"), &[])
}

/// `m` installed in `svc` and returned with its hash.
fn install(svc: &JobTemplates, tmp: &std::path::Path, m: &Value) -> String {
    let id = m["id"].as_str().unwrap();
    let src = write_template(&tmp.join("src"), id, m, BRIEF);
    std::fs::remove_file(src.join("brief.md")).unwrap();
    svc.install(&src, true).unwrap();
    svc.list().into_iter().find(|t| t.id == id).unwrap().hash
}

#[test]
fn a_cloud_manifest_parses_and_says_what_it_expects() {
    let pkg = parse(&cloud(ID, json!({}))).unwrap();
    let m = pkg.manifest;
    assert_eq!((m.kind, m.owner.as_deref(), m.max_duration_minutes), (JobTemplateKind::Cloud, Some("ops"), Some(120)));
    assert!(matches!(m.expect, Some(CloudExpect::Schedule(ref s)) if s.cron == "0 2 * * *" && s.grace_minutes == 30));
    assert_eq!(m.run_now.unwrap().link.as_deref(), Some("https://ci.example.com/run/nightly"));
    assert!(!m.aggregate);

    // A job that events start has no schedule; a service that reports a summary says so, and asks its owner instead.
    let events = cloud(ID, json!({ "expect": "none", "aggregate": true, "runNow": { "askOwner": "Run the nightly workflow" } }));
    let m = parse(&events).unwrap().manifest;
    assert!(matches!(m.expect, Some(CloudExpect::Never(_))) && m.aggregate);
    let again: Value = serde_json::to_value(&m).unwrap();
    assert_eq!((&again["expect"], &again["aggregate"]), (&json!("none"), &json!(true)), "it reads back as written");
    assert!(again.get("reporter").is_some() && again["runNow"].get("link").is_none());
}

#[test]
fn a_bad_cloud_field_says_which() {
    let key = test_keys::public_key(1);
    let cases: Vec<(Value, &str)> = vec![
        (json!({ "owner": null }), "names its owner"),
        (json!({ "owner": "  " }), "owner must be"),
        (json!({ "reporter": null }), "names its reporter"),
        (json!({ "reporter": { "name": "Acme CI", "key": key } }), "reporter.name"),
        (json!({ "reporter": { "name": "acme-ci", "key": "AAAA" } }), "reporter.key"),
        (json!({ "reporter": { "name": "acme-ci", "key": format!(" {key}
") } }), "reporter.key"),
        (json!({ "reporter": { "name": "acme-ci", "key": key, "extra": 1 } }), "unknown field"),
        (json!({ "expect": null }), "what to expect"),
        (json!({ "expect": "sometimes" }), "CloudExpect"),
        (json!({ "expect": { "cron": "not a cron", "tz": "UTC", "graceMinutes": 5 } }), "invalid cron"),
        (json!({ "expect": { "cron": "0 2 * * *", "tz": "Mars/Base", "graceMinutes": 5 } }), "invalid cron"),
        (json!({ "expect": { "cron": "0 2 * * *", "tz": "UTC", "graceMinutes": 1441 } }), "graceMinutes"),
        (json!({ "maxDurationMinutes": null }), "how long a run may take"),
        (json!({ "maxDurationMinutes": 0 }), "maxDurationMinutes is 1 to"),
        (json!({ "maxDurationMinutes": 10081 }), "maxDurationMinutes is 1 to"),
        (json!({ "runNow": { "link": "http://ci.example.com" } }), "https address"),
        (json!({ "runNow": { "link": "https://ci.example.com/a b" } }), "https address"),
        (json!({ "runNow": { "askOwner": "" } }), "askOwner"),
        (json!({ "runNow": { "link": "https://ci.example.com", "askOwner": "run it" } }), "one of link or askOwner"),
        (json!({ "runNow": {} }), "one of link or askOwner"),
    ];
    for (extra, says) in cases {
        let err = parse(&cloud(ID, extra.clone())).unwrap_err();
        assert!(err.contains(says), "{extra}: {err}");
    }
    assert!(parse(&cloud(ID, json!({ "runNow": null }))).is_ok(), "runNow is optional");
}

#[test]
fn only_a_cloud_template_carries_cloud_fields() {
    for extra in [
        json!({ "owner": "ops" }),
        json!({ "reporter": { "name": "acme-ci", "key": test_keys::public_key(1) } }),
        json!({ "expect": "none" }),
        json!({ "maxDurationMinutes": 5 }),
        json!({ "runNow": { "link": "https://ci.example.com" } }),
        json!({ "aggregate": true }),
    ] {
        let m = manifest_json("acme.drift", extra.clone());
        let files = [("template.json".to_string(), serde_json::to_vec(&m).unwrap()), ("brief.md".to_string(), BRIEF.as_bytes().to_vec())];
        let err = package::parse("acme.drift", &files.into_iter().collect()).unwrap_err();
        assert!(err.contains("belong to a cloud template"), "{extra}: {err}");
    }
    let plain = manifest_json("acme.drift", json!({}));
    let files = [("template.json".to_string(), serde_json::to_vec(&plain).unwrap()), ("brief.md".to_string(), BRIEF.as_bytes().to_vec())];
    assert!(package::parse("acme.drift", &files.into_iter().collect()).is_ok());
}

#[test]
fn approving_a_cloud_manifest_makes_it_a_job_and_pins_its_reporter() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = std::sync::Arc::new(service(tmp.path()));
    let hash = install(&svc, tmp.path(), &cloud(ID, json!({})));
    let key = test_keys::public_key(1);

    // Installed is not approved: the job is unknown and no reporter is pinned.
    assert_eq!(svc.cloud_job(ID), None);
    assert!(svc.reporters().list().is_empty());

    svc.approve(ID, &hash, "").unwrap();
    assert_eq!(svc.cloud_job(ID), Some(CloudJob { id: ID.into(), name: "Nightly".into(), reporter_key: key.clone() }));
    assert_eq!(svc.reporters().list().len(), 1);
    assert!(svc.reporters().fingerprint_of_pinned(&key).is_some());
    assert_eq!(svc.cloud_job("acme.other"), None, "an id no manifest names");
    // It survives a restart: the approval and the pin are on disk.
    let restarted = std::sync::Arc::new(service(tmp.path()));
    assert!(restarted.cloud_job(ID).is_some() && restarted.reporters().fingerprint_of_pinned(&key).is_some());

    // A change after approval drops the job until the operator approves again; uninstalling removes it for good.
    let mut changed = cloud(ID, json!({ "maxDurationMinutes": 240 }));
    changed["version"] = json!("1.0.1");
    std::fs::write(svc.templates_dir.join(ID).join("template.json"), serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(svc.cloud_job(ID), None);
    svc.uninstall(ID).unwrap();
    assert_eq!(svc.cloud_job(ID), None);
}

#[test]
fn a_reporter_name_pinned_to_another_key_stops_the_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = std::sync::Arc::new(service(tmp.path()));
    svc.reporters().add("acme-ci", &test_keys::public_key(2)).unwrap();
    let hash = install(&svc, tmp.path(), &cloud(ID, json!({})));

    let err = svc.approve(ID, &hash, "").unwrap_err();
    assert!(err.contains("reporter can't be pinned") && err.contains("forget it first"), "{err}");
    assert_eq!(svc.cloud_job(ID), None, "nothing was approved");

    svc.reporters().forget("acme-ci").unwrap();
    svc.approve(ID, &hash, "").unwrap();
    assert!(svc.cloud_job(ID).is_some());
}

#[test]
fn a_job_template_is_never_a_cloud_job() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = std::sync::Arc::new(service(tmp.path()));
    let src = write_template(&tmp.path().join("src"), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    svc.install(&src, false).unwrap();
    let hash = svc.list().into_iter().find(|t| t.id == "acme.drift").unwrap().hash;
    svc.approve("acme.drift", &hash, "").unwrap();
    assert!(svc.resolve_approved("acme.drift", "1.0.0").is_ok());
    assert_eq!(svc.cloud_job("acme.drift"), None, "events for a job template's id are dropped as an unknown job");
    assert!(svc.reporters().list().is_empty());
}

#[test]
fn the_prompt_is_told_what_approving_does_to_the_pins() {
    use crate::backend::jobs::reporters::ReporterState;
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let hash = install(&svc, tmp.path(), &cloud(ID, json!({})));
    let note = |svc: &JobTemplates| svc.list().into_iter().find(|t| t.id == ID).unwrap().reporter.unwrap();
    let key = test_keys::public_key(1);

    let n = note(&svc);
    assert_eq!((n.name.as_str(), n.fingerprint.as_str(), n.state), ("acme-ci", crate::backend::widget_signature::fingerprint(&key).as_str(), ReporterState::New));

    // The name is pinned to another key, or the key to another name: approving would be refused.
    svc.reporters().add("acme-ci", &test_keys::public_key(2)).unwrap();
    assert_eq!(note(&svc).state, ReporterState::NameTaken);
    svc.reporters().forget("acme-ci").unwrap();
    svc.reporters().add("other", &key).unwrap();
    assert_eq!(note(&svc).state, ReporterState::KeyTaken);
    assert!(svc.approve(ID, &hash, "").is_err());
    svc.reporters().forget("other").unwrap();

    svc.approve(ID, &hash, "").unwrap();
    assert_eq!(note(&svc).state, ReporterState::Pinned);

    // A job template has no reporter to show.
    let src = write_template(&tmp.path().join("src"), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    svc.install(&src, false).unwrap();
    assert!(svc.list().into_iter().find(|t| t.id == "acme.drift").unwrap().reporter.is_none());
}
