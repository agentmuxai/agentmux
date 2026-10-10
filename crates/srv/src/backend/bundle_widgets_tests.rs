// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `bundle_widgets.rs`.

use std::io::Write;

use super::*;

/// A zip of `entries` (name → bytes).
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        w.start_file(*name, opts).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap();
    buf.into_inner()
}

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0, 0xff, 0xfe, 1, 2, 3];

fn manifest(id: &str, kind: &str) -> String {
    serde_json::json!({
        "manifestVersion": 1, "id": id, "name": "Board", "version": "1.0.0", "kind": kind,
        "entry": if kind == "trusted" { "index.js" } else { "index.html" },
        "contributes": { "panes": [{ "name": "main" }] }
    })
    .to_string()
}

fn widget(id: &str, kind: &str) -> BundleWidget {
    let mut files = BTreeMap::new();
    files.insert(wp::MANIFEST_FILE.to_string(), manifest(id, kind).into_bytes());
    files.insert(if kind == "trusted" { "index.js" } else { "index.html" }.to_string(), b"<p>board</p>".to_vec());
    files.insert("img/logo.png".to_string(), PNG.to_vec());
    BundleWidget { id: id.to_string(), files }
}

fn hash_of(w: &BundleWidget) -> String {
    let (_tmp, root) = stage(w).unwrap();
    wp::package_hash(&wp::hash_files(&root).unwrap())
}

fn no_signature(id: &str, signer: Option<&str>) -> WidgetSignatureInfo {
    sig::describe(id, signer, &Default::default())
}

#[test]
fn widget_paths_are_the_ones_under_widgets() {
    assert!(is_widget_path("widgets/acme.board/widget.json"));
    assert!(!is_widget_path("widgets.md") && !is_widget_path("instructions/widgets/x") && !is_widget_path("widgetsx/a"));
}

#[test]
fn a_bundles_widgets_are_read_as_bytes_with_or_without_a_wrapper_folder() {
    let m = manifest("acme.board", "sandboxed");
    for prefix in ["my-bundle/", ""] {
        let zip = zip_of(&[
            (&format!("{prefix}bundle.json"), b"{}"),
            (&format!("{prefix}instructions/AGENTS.md"), b"hi"),
            (&format!("{prefix}widgets/acme.board/widget.json"), m.as_bytes()),
            (&format!("{prefix}widgets/acme.board/img/logo.png"), PNG),
        ]);
        let widgets = read_from_zip(&zip).unwrap();
        assert_eq!(widgets.len(), 1, "{prefix}");
        assert_eq!(widgets[0].id, "acme.board");
        assert_eq!(widgets[0].files["img/logo.png"], PNG, "binary bytes intact");
        assert!(!widgets[0].files.contains_key("instructions/AGENTS.md"));
    }
}

#[test]
fn a_widget_path_that_leaves_its_folder_or_isnt_an_id_is_refused() {
    let m = manifest("acme.board", "sandboxed");
    assert!(read_from_zip(&zip_of(&[("bundle.json", b"{}"), ("widgets/acme.board/../evil.txt", b"x"), ("widgets/acme.board/widget.json", m.as_bytes())])).is_err());
    assert!(read_from_zip(&zip_of(&[("bundle.json", b"{}"), ("widgets/NotAnId/widget.json", m.as_bytes())])).is_err());
    assert!(read_from_zip(&zip_of(&[("bundle.json", b"{}"), ("widgets/loose.txt", b"x")])).is_err());
}

#[test]
fn the_preview_checks_kind_and_the_hash_bundle_json_declares() {
    let w = widget("acme.board", "sandboxed");
    let hash = hash_of(&w);
    let declared = BTreeMap::from([("acme.board".to_string(), hash.clone())]);
    let p = preview(&w, &declared, &[], no_signature);
    assert_eq!(p.error, None);
    assert_eq!((p.name.as_str(), p.version.as_str(), p.hash.as_str()), ("Board", "1.0.0", hash.as_str()));
    assert_eq!(p.signature.unwrap().state, sig::SignatureState::Unsigned);

    let wrong = BTreeMap::from([("acme.board".to_string(), "0".repeat(64))]);
    assert!(preview(&w, &wrong, &[], no_signature).error.unwrap().contains("don't match"));
    assert!(preview(&w, &BTreeMap::new(), &[], no_signature).error.unwrap().contains("components.widgets"));

    let t = widget("acme.tool", "trusted");
    let declared = BTreeMap::from([("acme.tool".to_string(), hash_of(&t))]);
    assert!(preview(&t, &declared, &[], no_signature).error.unwrap().contains("sandboxed widgets only"));
}

#[test]
fn bundle_json_lists_its_widgets() {
    let d = declared(Some(r#"{"components":{"widgets":[{"id":"acme.board","version":"1.0.0","hash":"abc"}]}}"#));
    assert_eq!(d.get("acme.board").map(String::as_str), Some("abc"));
    assert!(declared(Some("{}")).is_empty() && declared(None).is_empty());
}

#[test]
fn installing_copies_the_package_in_as_one_waiting_for_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let widgets_dir = tmp.path().join("widgets");
    let w = widget("acme.board", "sandboxed");
    assert_eq!(install(&widgets_dir, &w).unwrap(), "acme.board");
    assert_eq!(std::fs::read(widgets_dir.join("acme.board/img/logo.png")).unwrap(), PNG);
    let svc = wp::WidgetPackages::new(widgets_dir, &tmp.path().join("data"), "secret".into());
    let p = &svc.rescan(&Default::default())[0];
    assert_eq!(p.state, WidgetState::NeedsApproval, "a bundle never approves a widget");
    assert_eq!(p.hash, hash_of(&w));
}
