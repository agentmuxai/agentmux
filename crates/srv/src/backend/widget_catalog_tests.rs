// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `widget_catalog.rs`.

use std::io::Write;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};

use super::*;

const URL: &str = "https://catalog.example/widgets/index.json";

fn key(seed: u8) -> (SigningKey, String) {
    let k = SigningKey::from_bytes(&[seed; 32]);
    let pk = base64::engine::general_purpose::STANDARD.encode(k.verifying_key().as_bytes());
    (k, pk)
}

fn signed(raw: &str, k: &SigningKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(k.sign(raw.as_bytes()).to_bytes())
}

fn entry(zip: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "acme.board", "name": "Board", "version": "1.0.0", "permissions": [],
        "hash": "h", "publisherKey": "k", "zip": zip, "zipSha256": "s"
    })
}

fn index(entries: Vec<serde_json::Value>) -> String {
    serde_json::json!({ "catalogVersion": 1, "generated": "2026-10-11T00:00:00Z", "widgets": entries }).to_string()
}

#[test]
fn only_an_index_signed_by_a_pinned_key_is_read() {
    let (k, pk) = key(1);
    let (other, _) = key(2);
    let raw = index(vec![entry("https://catalog.example/widgets/acme.board-1.0.0.zip")]);
    let list = verify_index(raw.as_bytes(), &signed(&raw, &k), &[&pk], URL).unwrap();
    assert_eq!(list[0].id, "acme.board");
    assert!(verify_index(raw.as_bytes(), &signed(&raw, &other), &[&pk], URL).unwrap_err().contains("doesn't verify"));
    // A changed byte breaks it.
    let changed = raw.replace("Board", "Bored");
    assert!(verify_index(changed.as_bytes(), &signed(&raw, &k), &[&pk], URL).is_err());
    // No key pinned: nothing verifies.
    assert!(verify_index(raw.as_bytes(), &signed(&raw, &k), &[], URL).is_err());
}

#[test]
fn a_download_from_anywhere_but_the_catalog_is_refused() {
    let (k, pk) = key(1);
    for zip in ["https://evil.example/acme.board.zip", "http://catalog.example/widgets/a.zip", "https://catalog.example@evil.example/a.zip"] {
        let raw = index(vec![entry(zip)]);
        assert!(verify_index(raw.as_bytes(), &signed(&raw, &k), &[&pk], URL).unwrap_err().contains("isn't served from the catalog"), "{zip}");
    }
    let raw = index(vec![serde_json::json!({ "id": "Bad Id", "name": "x", "version": "1.0.0", "hash": "h", "publisherKey": "k", "zip": "https://catalog.example/a.zip", "zipSha256": "s" })]);
    assert!(verify_index(raw.as_bytes(), &signed(&raw, &k), &[&pk], URL).is_err());
}

/// A signed sandboxed package as a zip, and its catalog entry.
fn package_zip(seed: u8, kind: &str) -> (Vec<u8>, CatalogEntry) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("acme.board");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("widget.json"),
        serde_json::json!({ "manifestVersion": 1, "id": "acme.board", "name": "Board", "version": "1.0.0", "kind": kind, "contributes": { "panes": [{ "name": "main" }] } }).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("index.html"), "<p>board</p>").unwrap();
    let hash = wp::package_hash(&wp::hash_files(&dir).unwrap());
    std::fs::write(dir.join(sig::SIG_FILE), sig::test_keys::sig_file(seed, "acme.board", "1.0.0", &hash)).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    for name in ["widget.json", "index.html", sig::SIG_FILE] {
        w.start_file(format!("acme.board/{name}"), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&std::fs::read(dir.join(name)).unwrap()).unwrap();
    }
    w.finish().unwrap();
    let zip = buf.into_inner();
    let entry = CatalogEntry {
        id: "acme.board".into(),
        name: "Board".into(),
        version: "1.0.0".into(),
        description: None,
        author: None,
        homepage: None,
        icon: None,
        permissions: vec![],
        hash,
        publisher_key: sig::test_keys::public_key(seed),
        zip: "https://catalog.example/widgets/acme.board-1.0.0.zip".into(),
        zip_sha256: hex::encode(Sha256::digest(&zip)),
    };
    (zip, entry)
}

#[test]
fn a_download_must_match_the_index_in_every_way() {
    let (zip, entry) = package_zip(3, "sandboxed");
    let (_tmp, root) = check_download(&zip, &entry).unwrap();
    assert!(root.join("widget.json").exists());

    let mut bad = entry.clone();
    bad.zip_sha256 = "0".repeat(64);
    assert!(check_download(&zip, &bad).unwrap_err().contains("download doesn't match"));

    // The zip is what the index says, but the index lists other files.
    let mut bad = entry.clone();
    bad.hash = "0".repeat(64);
    assert!(check_download(&zip, &bad).unwrap_err().contains("files don't match"));

    // Signed by someone other than the publisher the catalog lists.
    let mut bad = entry.clone();
    bad.publisher_key = sig::test_keys::public_key(4);
    assert!(check_download(&zip, &bad).unwrap_err().contains("isn't signed by the publisher"));

    let (zip, entry) = package_zip(3, "trusted");
    assert!(check_download(&zip, &entry).is_err());
}

#[test]
fn an_installed_widget_is_current_only_at_the_catalogs_files() {
    let (_zip, entry) = package_zip(3, "sandboxed");
    let tmp = tempfile::tempdir().unwrap();
    let svc = wp::WidgetPackages::new(tmp.path().join("widgets"), &tmp.path().join("data"), "s".into());
    assert!(!items(vec![entry.clone()], &svc.rescan(&Default::default()))[0].current);
    let (zip, entry2) = package_zip(3, "sandboxed");
    let (_t, root) = check_download(&zip, &entry2).unwrap();
    wp::install(&svc.widgets_dir, &root, true).unwrap();
    let listed = items(vec![entry], &svc.rescan(&Default::default()));
    assert!(listed[0].current);
    assert_eq!(listed[0].installed_state, Some(WidgetState::NeedsApproval), "the catalog never approves");
    assert_eq!(listed[0].fingerprint, sig::fingerprint(&sig::test_keys::public_key(3)));
}
