// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of job templates (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §8,
//! "Templates").

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::manifest::{self, JobTemplateManifest};
use super::*;
use crate::backend::widget_signature::{test_keys, SignatureState};

pub(super) fn manifest_json(id: &str, extra: Value) -> Value {
    let mut m = json!({
        "templateVersion": 1,
        "id": id,
        "name": "Docs drift",
        "version": "1.0.0",
        "kind": "job",
        "description": "Find docs whose claims no longer match the code.",
        "params": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "title": "Repository" },
                "doc_globs": { "type": "array", "items": { "type": "string" }, "default": ["docs/**"] },
                "focus": { "type": "string", "enum": ["docs", "security"], "default": "docs" },
                "max_rounds": { "type": "number", "default": 2 },
                "fix": { "type": "boolean", "default": false }
            },
            "required": ["repo"]
        },
        "trigger": { "kind": "cron", "cron": "0 6 * * 1", "tz": "UTC" },
        "action": { "kind": "message" },
        "policy": { "overlap": "skip", "catch_up": "latest" },
        "outputs": ["kanban_task", "report_file"]
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        if v.is_null() {
            m.as_object_mut().unwrap().remove(&k);
        } else {
            m[k] = v;
        }
    }
    m
}

pub(super) const BRIEF: &str = "Check {{repo}} ({{doc_globs}}) for {{focus}} drift, {{max_rounds}} rounds, fix: {{fix}}.";

/// A template folder named `id` in `parent`.
pub(super) fn write_template(parent: &Path, id: &str, m: &Value, brief: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("template.json"), serde_json::to_vec_pretty(m).unwrap()).unwrap();
    std::fs::write(dir.join("brief.md"), brief).unwrap();
    dir
}

/// Signs the template in `dir` with the key from `seed`, in `domain`.
pub(super) fn sign(dir: &Path, domain: &sig::Domain, seed: u8) {
    let pkg = package::parse(dir.file_name().unwrap().to_str().unwrap(), &package::read_files(dir).unwrap()).unwrap();
    let text = test_keys::sig_file_in(domain, seed, &pkg.manifest.id, &pkg.manifest.version, &pkg.hash);
    std::fs::write(dir.join("template.sig"), text).unwrap();
}

fn service(tmp: &Path) -> JobTemplates {
    JobTemplates::new(tmp.join("job-templates"), &tmp.join("data"), &[])
}

fn parse(m: &Value, brief: &str) -> Result<package::Package, String> {
    let id = m["id"].as_str().unwrap_or("acme.x").to_string();
    let files = [("template.json".to_string(), serde_json::to_vec(m).unwrap()), ("brief.md".to_string(), brief.as_bytes().to_vec())];
    package::parse(&id, &files.into_iter().collect())
}

fn params(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

#[test]
fn a_valid_template_parses_and_a_bad_one_says_why() {
    assert!(parse(&manifest_json("acme.drift", json!({})), BRIEF).is_ok());
    let refused = |extra: Value, brief: &str, why: &str| {
        let err = parse(&manifest_json("acme.drift", extra.clone()), brief).unwrap_err();
        assert!(err.contains(why), "{extra}: {err}");
    };
    refused(json!({ "templateVersion": 2 }), BRIEF, "templateVersion");
    refused(json!({ "version": "1.0" }), BRIEF, "semver");
    // A misspelt field is refused, never silently dropped: a lost "secrets"
    // would hide them from the Rule of Two.
    refused(json!({ "secret": [] }), BRIEF, "unknown field");
    refused(json!({ "action": { "kind": "drone" } }), BRIEF, "later version");
    refused(json!({ "action": { "kind": "message", "brief": "hi" } }), BRIEF, "brief.md");
    refused(json!({ "action": null }), BRIEF, "needs an action");
    refused(json!({ "trigger": { "kind": "hourly" } }), BRIEF, "trigger.kind");
    refused(json!({ "outputs": ["email"] }), BRIEF, "unknown output");
    refused(json!({ "policy": { "approval": "maybe" } }), BRIEF, "policy.approval");
    refused(json!({ "params": { "properties": { "x": { "type": "object" } } } }), "", "parameter \"x\"");
    refused(json!({ "params": { "properties": { "Repo": { "type": "string" } } } }), "", "parameter name");
    refused(json!({}), "Check {{repos}}.", "isn't one of its parameters");
    refused(json!({}), "Check {{ repo }}.", "line 1");
    refused(json!({}), "Line one\nCheck {{repo.", "line 2");
    refused(json!({ "secrets": [{ "name": "TOKEN", "param": "repo" }] }), BRIEF, "credential");
    refused(
        json!({ "params": { "properties": { "t": { "type": "string", "format": "credential" } } }, "secrets": [{ "name": "AGENTMUX_T", "param": "t" }] }),
        "",
        "AGENTMUX_",
    );
    assert!(parse(&manifest_json("acme.drift", json!({})), BRIEF).is_ok_and(|p| p.manifest.id == "acme.drift"));
}

#[test]
fn a_package_holds_only_its_known_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_template(tmp.path(), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    assert!(package::load_dir(&dir, "acme.drift").is_ok());
    assert!(package::load_dir(&dir, "acme.other").unwrap_err().contains("folder name"));
    std::fs::write(dir.join("graph.json"), "{}").unwrap();
    assert!(package::load_dir(&dir, "acme.drift").unwrap_err().contains("later version"), "a Drone graph waits for P4");
    std::fs::remove_file(dir.join("graph.json")).unwrap();
    std::fs::write(dir.join("run.sh"), "rm -rf /").unwrap();
    assert!(package::load_dir(&dir, "acme.drift").unwrap_err().contains("run.sh"));
    std::fs::remove_file(dir.join("run.sh")).unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    assert!(package::load_dir(&dir, "acme.drift").unwrap_err().contains("folder"));
}

#[test]
fn a_changed_file_changes_the_hash_and_the_signature_file_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_template(tmp.path(), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    let first = package::load_dir(&dir, "acme.drift").unwrap().hash;
    sign(&dir, &sig::JOB_TEMPLATE, 3);
    assert_eq!(package::load_dir(&dir, "acme.drift").unwrap().hash, first, "template.sig is left out of the hash");
    std::fs::write(dir.join("brief.md"), format!("{BRIEF} Also look at the README.")).unwrap();
    std::fs::remove_file(dir.join("template.sig")).unwrap();
    assert_ne!(package::load_dir(&dir, "acme.drift").unwrap().hash, first);
}

#[test]
fn a_template_signature_verifies_with_the_template_prefix_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_template(tmp.path(), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    sign(&dir, &sig::JOB_TEMPLATE, 3);
    assert_eq!(package::load_dir(&dir, "acme.drift").unwrap().signer, Some(test_keys::public_key(3)));
    // The same key over the same id, version and hash, but as a widget.
    sign(&dir, &sig::WIDGET, 3);
    assert!(package::load_dir(&dir, "acme.drift").unwrap_err().contains("doesn't match"));
}

/// The template the SDK's `agentmux-widget sign` signed with the test seed
/// (frontend/app/block/widget-sign.test.ts signs it again): both sides agree
/// on its hash and its signature.
#[test]
fn the_sdk_signed_template_fixture_verifies_here() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/widget-sdk/fixtures/acme.template-fixture");
    let pkg = package::load_dir(&dir, "acme.template-fixture").unwrap();
    assert_eq!(pkg.hash, "9c78ceeea2cf6db5e70cde3521bc24c971a7ef39fd73afb9e1050b7a7c89b761");
    assert_eq!(pkg.signer, Some(test_keys::public_key(7)));
}

#[test]
fn a_brief_renders_its_parameters_as_data() {
    let m: JobTemplateManifest = serde_json::from_value(manifest_json("acme.drift", json!({}))).unwrap();
    let r = manifest::render(&m, BRIEF, &params(json!({ "repo": "acme/site" }))).unwrap();
    assert_eq!(r.brief, "Check acme/site (docs/**) for docs drift, 2 rounds, fix: false.");
    assert_eq!(r.params["doc_globs"], json!(["docs/**"]), "defaults are filled in");
    let r = manifest::render(&m, BRIEF, &params(json!({ "repo": "{{focus}}", "doc_globs": ["a", "b"], "fix": true }))).unwrap();
    assert_eq!(r.brief, "Check {{focus}} (a, b) for docs drift, 2 rounds, fix: true.", "a value is never expanded again");
    for (bad, why) in [
        (json!({}), "required"),
        (json!({ "repo": "x", "branch": "main" }), "no parameter \"branch\""),
        (json!({ "repo": 7 }), "isn't a valid string"),
        (json!({ "repo": "x", "focus": "style" }), "choice"),
        (json!({ "repo": "x", "doc_globs": [1] }), "list of strings"),
    ] {
        let err = manifest::render(&m, BRIEF, &params(bad.clone())).unwrap_err();
        assert!(err.contains(why), "{bad}: {err}");
    }
}

#[test]
fn the_rule_of_two_refuses_untrusted_input_with_secrets_and_outside_writes() {
    let risky = json!({
        "trigger": { "kind": "event", "source": "github" },
        "params": { "properties": { "token": { "type": "string", "format": "credential" } } },
        "secrets": [{ "name": "SCANNER_TOKEN", "param": "token" }],
        "outputs": ["pull_request", "report_file"]
    });
    let err = parse(&manifest_json("acme.watch", risky.clone()), "").unwrap_err();
    assert!(err.contains("untrusted") && err.contains("pull_request"), "{err}");
    // Any one of the three gone, or an approval per run, and it passes.
    let mut with_approval = risky.clone();
    with_approval["policy"] = json!({ "approval": "required" });
    assert!(parse(&manifest_json("acme.watch", with_approval), "").is_ok());
    let mut no_outside = risky.clone();
    no_outside["outputs"] = json!(["report_file", "message:owner"]);
    assert!(parse(&manifest_json("acme.watch", no_outside), "").is_ok());
    let mut no_secrets = risky.clone();
    no_secrets["secrets"] = json!([]);
    no_secrets["params"] = json!({});
    assert!(parse(&manifest_json("acme.watch", no_secrets), "").is_ok());
    // A credential parameter is a secret even when no secret names it.
    let mut credential_only = risky.clone();
    credential_only["secrets"] = json!([]);
    assert!(parse(&manifest_json("acme.watch", credential_only), "").is_err());
    // A manual trigger that reads untrusted text is held to the same rule.
    let mut declared = risky.clone();
    declared["trigger"] = json!({ "kind": "manual" });
    declared["readsUntrusted"] = json!(true);
    assert!(parse(&manifest_json("acme.watch", declared.clone()), "").is_err());
    declared["readsUntrusted"] = json!(false);
    assert!(parse(&manifest_json("acme.watch", declared), "").is_ok());
}

#[test]
fn an_unapproved_or_changed_template_cannot_be_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let src = write_template(&tmp.path().join("src"), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    assert_eq!(svc.install(&src, false).unwrap(), "acme.drift");
    let info = svc.list().into_iter().find(|t| t.id == "acme.drift").unwrap();
    assert_eq!(info.state, JobTemplateState::NeedsApproval);
    assert!(svc.resolve_approved("acme.drift", "1.0.0").unwrap_err().contains("isn't approved"));

    assert!(svc.approve("acme.drift", "stale", "").unwrap_err().contains("changed since you were asked"));
    assert!(svc.approve("acme.drift", &info.hash, "AAAA-BBBB").unwrap_err().contains("signature changed"));
    svc.approve("acme.drift", &info.hash, "").unwrap();
    assert_eq!(svc.list()[0].state, JobTemplateState::Approved);
    let t = svc.resolve_approved("acme.drift", "1.0.0").unwrap();
    assert_eq!(t.hash, info.hash);
    assert_eq!(t.render(&params(json!({ "repo": "acme/site" }))).unwrap().brief.split(" (").next(), Some("Check acme/site"));
    assert!(svc.resolve_approved("acme.drift", "2.0.0").unwrap_err().contains("not 2.0.0"));

    // An edit after approval needs approval again; so does a reinstall.
    std::fs::write(svc.templates_dir.join("acme.drift/brief.md"), "Check {{repo}} and push to main.").unwrap();
    assert_eq!(svc.list()[0].state, JobTemplateState::Changed);
    assert!(svc.resolve_approved("acme.drift", "1.0.0").unwrap_err().contains("changed since"));
    assert!(svc.install(&src, false).unwrap_err().contains("already installed"));
    svc.install(&src, true).unwrap();
    assert_eq!(svc.list()[0].state, JobTemplateState::Approved, "the approved files are back");

    // A fresh service reads the approval from disk.
    assert!(service(tmp.path()).resolve_approved("acme.drift", "1.0.0").is_ok());
    svc.uninstall("acme.drift").unwrap();
    assert!(svc.resolve_approved("acme.drift", "1.0.0").unwrap_err().contains("no job template"));
}

#[test]
fn a_changed_publisher_key_shows_as_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let src = tmp.path().join("src");
    let a = write_template(&src, "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    sign(&a, &sig::JOB_TEMPLATE, 1);
    svc.install(&a, false).unwrap();
    let info = svc.get("acme.drift").unwrap().0;
    assert_eq!(info.signature.state, SignatureState::SignedNew);
    svc.approve("acme.drift", &info.hash, info.signature.fingerprint.as_deref().unwrap()).unwrap();
    assert_eq!(svc.publishers()[0].fingerprint, sig::fingerprint(&test_keys::public_key(1)), "pinned on first approval");
    assert!(std::fs::read_to_string(tmp.path().join("data/job-template-publishers.json")).unwrap().contains(&test_keys::public_key(1)));

    // Another template of the same publisher, signed by another key.
    let b = write_template(&src, "acme.sweep", &manifest_json("acme.sweep", json!({})), BRIEF);
    sign(&b, &sig::JOB_TEMPLATE, 2);
    svc.install(&b, false).unwrap();
    assert_eq!(svc.get("acme.sweep").unwrap().0.signature.state, SignatureState::KeyChanged);

    // The approved template, re-signed by the other key: changed, and it
    // can't make a job until approved again.
    sign(&a, &sig::JOB_TEMPLATE, 2);
    svc.install(&a, true).unwrap();
    let info = svc.get("acme.drift").unwrap().0;
    assert_eq!((info.state, info.signature.state), (JobTemplateState::Changed, SignatureState::KeyChanged));
    assert!(svc.resolve_approved("acme.drift", "1.0.0").is_err());
    // Approving it again doesn't move the pin.
    svc.approve("acme.drift", &info.hash, info.signature.fingerprint.as_deref().unwrap()).unwrap();
    assert_eq!(svc.publishers()[0].fingerprint, sig::fingerprint(&test_keys::public_key(1)));
}

static TEST_BUILTINS: &[Builtin] = &[Builtin {
    id: "agentmux.hello",
    files: &[
        (
            "template.json",
            br#"{ "templateVersion": 1, "id": "agentmux.hello", "name": "Hello", "version": "1.0.0", "kind": "job",
                 "description": "Say hello.", "params": { "properties": { "who": { "type": "string" } }, "required": ["who"] },
                 "trigger": { "kind": "manual" }, "action": { "kind": "message" }, "outputs": ["message:owner"] }"#,
        ),
        ("brief.md", b"Say hello to {{who}}."),
    ],
}];

#[test]
fn built_in_templates_are_published_by_agentmux_and_still_approved_once() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = JobTemplates::new(tmp.path().join("job-templates"), &tmp.path().join("data"), TEST_BUILTINS);
    let info = svc.list().into_iter().find(|t| t.id == "agentmux.hello").unwrap();
    assert_eq!((info.source, info.state, info.signature.publisher.as_str()), (JobTemplateSource::Builtin, JobTemplateState::NeedsApproval, "agentmux"));
    assert!(svc.resolve_approved("agentmux.hello", "1.0.0").is_err());
    svc.approve("agentmux.hello", &info.hash, "").unwrap();
    let t = svc.resolve_approved("agentmux.hello", "1.0.0").unwrap();
    assert_eq!(t.render(&params(json!({ "who": "the team" }))).unwrap().brief, "Say hello to the team.");
    assert!(svc.uninstall("agentmux.hello").is_err());

    // Nobody else publishes as AgentMux: not by install, not by a folder
    // dropped in beside the others.
    let src = write_template(&tmp.path().join("src"), "agentmux.fake", &manifest_json("agentmux.fake", json!({})), BRIEF);
    assert!(svc.install(&src, false).unwrap_err().contains("kept for the templates AgentMux ships"));
    write_template(&svc.templates_dir, "agentmux.fake", &manifest_json("agentmux.fake", json!({})), BRIEF);
    let fake = svc.list().into_iter().find(|t| t.id == "agentmux.fake").unwrap();
    assert_eq!(fake.state, JobTemplateState::Invalid);
    assert!(svc.approve("agentmux.fake", &fake.hash, "").is_err());
}

/// Every folder in `templates/jobs/` is compiled in, from the same bytes,
/// and valid.
#[test]
fn the_shipped_templates_are_all_compiled_in() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/jobs");
    let mut on_disk: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut compiled: Vec<String> = BUILTINS.iter().map(|b| b.id.to_string()).collect();
    compiled.sort();
    assert_eq!(on_disk, compiled, "add each templates/jobs/ folder to BUILTINS");
    for b in BUILTINS {
        assert_eq!(sig::publisher_of(b.id), BUILTIN_PUBLISHER);
        let files = package::read_files(&root.join(b.id)).unwrap();
        assert_eq!(files.len(), b.files.len(), "{}: every file is compiled in", b.id);
        for (path, bytes) in b.files {
            assert_eq!(files.get(*path).map(Vec::as_slice), Some(*bytes), "{}/{path}", b.id);
        }
        package::load_embedded(b.id, b.files).unwrap();
    }
}
