// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of private job template catalogs, from fixtures: no network.

use std::io::Write;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;

use super::super::tests::{manifest_json, sign, write_template, BRIEF};
use super::*;

const URL: &str = "https://templates.example/jobs/index.json";

fn key(seed: u8) -> (SigningKey, String) {
    let k = SigningKey::from_bytes(&[seed; 32]);
    let pk = base64::engine::general_purpose::STANDARD.encode(k.verifying_key().as_bytes());
    (k, pk)
}

fn signed(raw: &str, k: &SigningKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(k.sign(raw.as_bytes()).to_bytes())
}

fn index(entries: Vec<serde_json::Value>) -> String {
    json!({ "catalogVersion": 1, "templates": entries }).to_string()
}

fn entry(id: &str, zip: &str) -> serde_json::Value {
    json!({ "id": id, "name": "Drift", "version": "1.0.0", "hash": "h", "publisherKey": "k", "zip": zip, "zipSha256": "s" })
}

#[test]
fn only_an_index_signed_by_the_pinned_key_is_read() {
    let (k, pk) = key(1);
    let (other, other_pk) = key(2);
    let catalog = Catalog::new(URL, &pk).unwrap();
    let raw = index(vec![entry("acme.drift", "https://templates.example/jobs/acme.drift-1.0.0.zip")]);
    assert_eq!(verify_index(raw.as_bytes(), &signed(&raw, &k), &catalog).unwrap()[0].id, "acme.drift");
    let err = verify_index(raw.as_bytes(), &signed(&raw, &other), &catalog).unwrap_err();
    assert!(err.contains("doesn't verify with the key you pinned"), "{err}");
    assert!(verify_index(raw.replace("Drift", "Dr1ft").as_bytes(), &signed(&raw, &k), &catalog).is_err(), "a changed byte breaks it");
    assert!(verify_index(raw.as_bytes(), "not base64!", &catalog).is_err());
    // Another key pinned for the same URL doesn't verify it.
    assert!(verify_index(raw.as_bytes(), &signed(&raw, &k), &Catalog::new(URL, &other_pk).unwrap()).is_err());
}

#[test]
fn a_catalog_entry_must_come_from_the_catalog_and_not_claim_agentmux() {
    let (k, pk) = key(1);
    let catalog = Catalog::new(URL, &pk).unwrap();
    for e in [
        entry("acme.drift", "https://evil.example/acme.drift.zip"),
        entry("acme.drift", "http://templates.example/jobs/a.zip"),
        entry("agentmux.drift", "https://templates.example/jobs/a.zip"),
        entry("Bad Id", "https://templates.example/jobs/a.zip"),
    ] {
        let raw = index(vec![e.clone()]);
        assert!(verify_index(raw.as_bytes(), &signed(&raw, &k), &catalog).is_err(), "{e}");
    }
    assert!(Catalog::new("http://templates.example/index.json", &pk).is_err());
    assert!(Catalog::new(URL, "c2hvcnQ=").is_err());
}

/// The template `acme.drift`, signed by the key from `seed`, as a zip, and
/// its catalog entry.
fn template_zip(seed: u8) -> (Vec<u8>, JobTemplateCatalogEntry) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_template(tmp.path(), "acme.drift", &manifest_json("acme.drift", json!({})), BRIEF);
    sign(&dir, &sig::JOB_TEMPLATE, seed);
    let pkg = package::load_dir(&dir, "acme.drift").unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    for name in ["template.json", "brief.md", "template.sig"] {
        w.start_file(format!("acme.drift/{name}"), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&std::fs::read(dir.join(name)).unwrap()).unwrap();
    }
    w.finish().unwrap();
    let zip = buf.into_inner();
    let entry = JobTemplateCatalogEntry {
        id: "acme.drift".into(),
        name: "Docs drift".into(),
        version: "1.0.0".into(),
        description: None,
        hash: pkg.hash,
        publisher_key: test_keys_public(seed),
        zip: "https://templates.example/jobs/acme.drift-1.0.0.zip".into(),
        zip_sha256: hex::encode(Sha256::digest(&zip)),
    };
    (zip, entry)
}

fn test_keys_public(seed: u8) -> String {
    crate::backend::widget_signature::test_keys::public_key(seed)
}

#[test]
fn a_download_installs_only_as_the_catalog_describes_it_and_waits_for_approval() {
    let (zip, entry) = template_zip(5);
    let (_tmp, root) = check_download(&zip, &entry).unwrap();
    let home = tempfile::tempdir().unwrap();
    let svc = super::super::JobTemplates::new(home.path().join("job-templates"), &home.path().join("data"), &[]);
    assert_eq!(svc.install(&root, true).unwrap(), "acme.drift");
    assert_eq!(svc.list()[0].state, super::super::JobTemplateState::NeedsApproval);

    let mut other = entry.clone();
    other.zip_sha256 = "0".repeat(64);
    assert!(check_download(&zip, &other).unwrap_err().contains("download doesn't match"));
    let mut other = entry.clone();
    other.hash = "0".repeat(64);
    assert!(check_download(&zip, &other).unwrap_err().contains("files don't match"));
    let mut other = entry.clone();
    other.publisher_key = test_keys_public(6);
    assert!(check_download(&zip, &other).unwrap_err().contains("isn't signed by the publisher"));
    let mut other = entry.clone();
    other.version = "1.0.1".into();
    assert!(check_download(&zip, &other).is_err());
}
