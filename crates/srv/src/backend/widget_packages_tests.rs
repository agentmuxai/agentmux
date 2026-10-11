// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `widget_packages.rs`.

use super::*;

fn manifest(id: &str) -> serde_json::Value {
    serde_json::json!({
        "manifestVersion": 1,
        "id": id,
        "name": "Test",
        "version": "1.0.0",
        "permissions": ["storage", "net:https://api.github.com"],
        "contributes": { "panes": [{ "name": "main" }] }
    })
}

fn package(root: &Path, id: &str) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MANIFEST_FILE), manifest(id).to_string()).unwrap();
    std::fs::write(dir.join("index.html"), "<p>hi</p>").unwrap();
    dir
}

fn service(root: &Path) -> WidgetPackages {
    WidgetPackages::new(root.join("widgets"), &root.join("data"), "secret".into())
}

#[test]
fn ids_versions_and_permissions_are_checked() {
    assert!(valid_id("acme.pr-dashboard"));
    assert!(!valid_id("Acme.x") && !valid_id("acme") && !valid_id("a.b.c") && !valid_id("acme."));
    assert!(valid_semver("1.2.0") && valid_semver("1.2.0-beta.1") && !valid_semver("1.2"));
    assert!(valid_permission("storage") && valid_permission("net:https://api.github.com"));
    assert!(valid_permission("net:https://*.example.com") && valid_permission("net:http://127.0.0.1:8188"));
    assert!(!valid_permission("net:https://*") && !valid_permission("net:https://a.com/path"));
    assert!(!valid_permission("net:ftp://a.com") && !valid_permission("everything"));
    assert!(!valid_permission("net:https://user@a.com"));
}

#[test]
fn a_manifest_must_match_its_folder_and_keep_meta_namespaced() {
    let mut m: Manifest = serde_json::from_value(manifest("acme.test")).unwrap();
    assert!(validate(&m, "acme.test").is_ok());
    assert!(validate(&m, "acme.other").unwrap_err().contains("folder"));
    m.contributes.panes[0].default_meta = Some(serde_json::Map::from_iter([("view".into(), "term".into())]));
    assert!(validate(&m, "acme.test").unwrap_err().contains("widget:"));
    m.contributes.panes[0].default_meta = None;
    m.contributes.panes[0].entry = Some("../escape.html".into());
    assert!(validate(&m, "acme.test").unwrap_err().contains("inside the package"));
}

#[test]
fn commands_and_status_items_are_checked() {
    let with = |contributes: serde_json::Value| {
        let mut v = manifest("acme.test");
        v["contributes"] = contributes;
        serde_json::from_value::<Manifest>(v).map_err(|e| e.to_string()).and_then(|m| validate(&m, "acme.test"))
    };
    let panes = serde_json::json!([{ "name": "main" }]);
    assert!(with(serde_json::json!({
        "panes": panes,
        "commands": [{ "id": "refresh", "title": "Refresh", "icon": "rotate" }],
        "statusItems": [{ "id": "count", "text": "PRs", "command": "refresh", "alignment": "left" }]
    }))
    .is_ok());
    let err = |c: serde_json::Value| with(c).unwrap_err();
    assert!(err(serde_json::json!({ "panes": panes, "commands": [{ "id": "Bad", "title": "x" }] })).contains("lowercase"));
    assert!(err(serde_json::json!({ "panes": panes, "commands": [{ "id": "a", "title": "" }] })).contains("title"));
    assert!(err(serde_json::json!({ "panes": panes, "commands": [{ "id": "a", "title": "x" }, { "id": "a", "title": "y" }] })).contains("two commands"));
    assert!(err(serde_json::json!({ "panes": panes, "commands": [{ "id": "a", "title": "x", "pane": "other" }] })).contains("doesn't contribute"));
    assert!(err(serde_json::json!({ "panes": panes, "commands": [{ "id": "a", "title": "x", "icon": "x\" onclick" }] })).contains("Font Awesome"));
    assert!(err(serde_json::json!({ "panes": panes, "statusItems": [{ "id": "s", "text": "x", "command": "nope" }] })).contains("doesn't contribute"));
    assert!(err(serde_json::json!({ "panes": panes, "statusItems": [{ "id": "s", "text": "a very long status bar text of more than forty" }] })).contains("1–40"));
    let five: Vec<_> = (0..5).map(|i| serde_json::json!({ "id": format!("s{i}"), "text": "x" })).collect();
    assert!(err(serde_json::json!({ "panes": panes, "statusItems": five })).contains("more than 4"));
    let many: Vec<_> = (0..21).map(|i| serde_json::json!({ "id": format!("c{i}"), "title": "x" })).collect();
    assert!(err(serde_json::json!({ "panes": panes, "commands": many })).contains("more than 20"));
}

#[test]
fn commands_and_status_items_reach_the_ui_with_their_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    let mut v = manifest("acme.test");
    v["icon"] = "note-sticky".into();
    v["contributes"] = serde_json::json!({
        "panes": [{ "name": "main" }, { "name": "side" }],
        "commands": [{ "id": "new", "title": "New note", "pane": "side", "keywords": "add" }, { "id": "show", "title": "Show" }],
        "statusItems": [{ "id": "count", "text": "Notes", "command": "show" }]
    });
    std::fs::write(dir.join(MANIFEST_FILE), v.to_string()).unwrap();
    let p = &svc.rescan(&HashMap::new())[0];
    assert_eq!(p.state, WidgetState::NeedsApproval, "{:?}", p.error);
    assert_eq!(p.commands[0].view, "ext:acme.test/side");
    assert_eq!((p.commands[1].view.as_str(), p.commands[1].icon.as_str()), ("ext:acme.test/main", "note-sticky"));
    assert_eq!(p.commands[0].keywords, "add");
    let s = &p.status_items[0];
    assert_eq!((s.text.as_str(), s.command.as_deref(), s.alignment), ("Notes", Some("show"), StatusAlignment::Right));
}

#[test]
fn a_new_package_needs_approval_and_approval_names_its_exact_files() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    package(&svc.widgets_dir, "acme.test");
    let p = &svc.rescan(&HashMap::new())[0];
    assert_eq!(p.state, WidgetState::NeedsApproval);
    assert_eq!(p.files_url, None);
    assert_eq!(p.panes[0].view, "ext:acme.test/main");

    svc.approve("acme.test", &p.hash, p.signature.fingerprint.as_deref().unwrap_or("")).unwrap();
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!(p.state, WidgetState::Approved);
    assert_eq!(p.granted, vec!["storage".to_string(), "net:https://api.github.com".to_string()]);
    let url = p.files_url.clone().unwrap();
    let key = url.trim_end_matches('/').rsplit('/').next().unwrap();
    assert_eq!(svc.read_file("acme.test", &p.hash, key, "index.html").unwrap(), b"<p>hi</p>");

    // An approval of a stale hash is refused.
    assert!(svc.approve("acme.test", "0000", "").is_err());
}

fn sign(dir: &Path, seed: u8, id: &str) {
    let hash = package_hash(&hash_files(dir).unwrap());
    std::fs::write(dir.join(sig::SIG_FILE), sig::test_keys::sig_file(seed, id, "1.0.0", &hash)).unwrap();
}

#[test]
fn a_signature_leaves_the_hash_alone_and_names_its_key() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    let unsigned = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!(unsigned.signature.state, sig::SignatureState::Unsigned);
    sign(&dir, 1, "acme.test");
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!(p.hash, unsigned.hash, "widget.sig isn't part of the content hash");
    assert_eq!(p.signature.state, sig::SignatureState::SignedNew);
    assert_eq!(p.signature.fingerprint.as_deref(), Some(sig::fingerprint(&sig::test_keys::public_key(1)).as_str()));

    // Approving pins the publisher; the package is then plainly signed.
    svc.approve("acme.test", &p.hash, p.signature.fingerprint.as_deref().unwrap_or("")).unwrap();
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!((p.state.clone(), p.signature.state), (WidgetState::Approved, sig::SignatureState::Signed));
    assert_eq!(svc.publishers().len(), 1);
    // The signature file is never served.
    let key = p.files_url.clone().unwrap().trim_end_matches('/').rsplit('/').next().unwrap().to_string();
    assert!(svc.read_file("acme.test", &p.hash, &key, sig::SIG_FILE).is_err());
}

#[test]
fn a_signature_that_doesnt_match_makes_the_package_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    sign(&dir, 1, "acme.test");
    std::fs::write(dir.join("index.html"), "<p>edited after signing</p>").unwrap();
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!(p.state, WidgetState::Invalid);
    assert!(p.error.unwrap().contains("signature"));
}

#[test]
fn another_key_for_a_pinned_publisher_is_flagged_and_asks_again() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    sign(&dir, 1, "acme.test");
    let p = svc.rescan(&HashMap::new())[0].clone();
    svc.approve("acme.test", &p.hash, p.signature.fingerprint.as_deref().unwrap_or("")).unwrap();

    // Same files, re-signed by someone else: asks again, flagged.
    sign(&dir, 2, "acme.test");
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!((p.state.clone(), p.signature.state), (WidgetState::Changed, sig::SignatureState::KeyChanged));
    // Approving it doesn't move the pin.
    svc.approve("acme.test", &p.hash, p.signature.fingerprint.as_deref().unwrap_or("")).unwrap();
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!((p.state.clone(), p.signature.state), (WidgetState::Approved, sig::SignatureState::KeyChanged));

    // Another acme widget, unsigned: flagged too.
    package(&svc.widgets_dir, "acme.other");
    let other = svc.rescan(&HashMap::new()).into_iter().find(|p| p.id == "acme.other").unwrap();
    assert_eq!(other.signature.state, sig::SignatureState::KeyChanged);

    // Forgetting the key: the next signed one pins again.
    svc.forget_publisher("acme").unwrap();
    assert!(svc.publishers().is_empty());
    assert!(svc.forget_publisher("acme").is_err());
}

#[test]
fn a_signature_swapped_after_the_prompt_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    sign(&dir, 1, "acme.test");
    let shown = svc.rescan(&HashMap::new())[0].clone();
    // Another key signs the same files before the click: the hash still
    // matches, the key the user saw doesn't.
    sign(&dir, 2, "acme.test");
    let err = svc.approve("acme.test", &shown.hash, shown.signature.fingerprint.as_deref().unwrap()).unwrap_err();
    assert!(err.contains("signature changed"), "{err}");
    assert!(svc.publishers().is_empty(), "nothing pinned");
    // Shown as unsigned, signed now: refused too.
    assert!(svc.approve("acme.test", &shown.hash, "").is_err());
}

#[test]
fn removing_the_signature_of_an_approved_package_asks_again() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    sign(&dir, 1, "acme.test");
    let p = svc.rescan(&HashMap::new())[0].clone();
    svc.approve("acme.test", &p.hash, p.signature.fingerprint.as_deref().unwrap_or("")).unwrap();
    std::fs::remove_file(dir.join(sig::SIG_FILE)).unwrap();
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!((p.state.clone(), p.signature.state), (WidgetState::Changed, sig::SignatureState::KeyChanged));
}

#[test]
fn an_edited_file_is_never_served_and_the_package_asks_again() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let dir = package(&svc.widgets_dir, "acme.test");
    let hash = svc.rescan(&HashMap::new())[0].hash.clone();
    svc.approve("acme.test", &hash, "").unwrap();
    let key = files_key("secret", "acme.test", &hash);

    // Edited after approval, before any rescan: the read itself catches it.
    std::fs::write(dir.join("index.html"), "<script>evil()</script>").unwrap();
    assert_eq!(svc.read_file("acme.test", &hash, &key, "index.html"), Err(FileError::Changed));
    // A file added after approval isn't served either.
    std::fs::write(dir.join("extra.js"), "x").unwrap();
    assert_eq!(svc.read_file("acme.test", &hash, &key, "extra.js"), Err(FileError::NotFound));
    // And the rescan marks it changed, with no files URL.
    let p = svc.rescan(&HashMap::new())[0].clone();
    assert_eq!(p.state, WidgetState::Changed);
    assert_eq!(p.files_url, None);
}

#[test]
fn files_need_the_right_key_and_stay_inside_the_package() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    package(&svc.widgets_dir, "acme.test");
    std::fs::write(svc.widgets_dir.join("secret.txt"), "nope").unwrap();
    let hash = svc.rescan(&HashMap::new())[0].hash.clone();
    svc.approve("acme.test", &hash, "").unwrap();
    let key = files_key("secret", "acme.test", &hash);
    assert_eq!(svc.read_file("acme.test", &hash, "guess", "index.html"), Err(FileError::NotFound));
    assert_eq!(svc.read_file("acme.test", &hash, &key, "../secret.txt"), Err(FileError::NotFound));
    assert_eq!(svc.read_file("acme.test", &hash, &key, "/etc/passwd"), Err(FileError::NotFound));
}

#[test]
fn disabling_stops_serving_and_removes_the_widget_bar_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    package(&svc.widgets_dir, "acme.test");
    let hash = svc.rescan(&HashMap::new())[0].hash.clone();
    svc.approve("acme.test", &hash, "").unwrap();
    let packages = svc.rescan(&HashMap::new());
    let entries = widget_entries(&packages);
    assert_eq!(entries["ext@acme.test/main"].block_def.meta["view"], "ext:acme.test/main");

    svc.set_enabled("acme.test", false).unwrap();
    let packages = svc.rescan(&HashMap::new());
    assert_eq!(packages[0].state, WidgetState::Disabled);
    assert!(widget_entries(&packages).is_empty());
    let key = files_key("secret", "acme.test", &hash);
    assert_eq!(svc.read_file("acme.test", &hash, &key, "index.html"), Err(FileError::NotFound));
}

#[test]
fn approvals_are_kept_per_instance_data_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    package(&svc.widgets_dir, "acme.test");
    let hash = svc.rescan(&HashMap::new())[0].hash.clone();
    svc.approve("acme.test", &hash, "").unwrap();
    // Same widgets folder, another instance's data dir: asks again.
    let other = WidgetPackages::new(tmp.path().join("widgets"), &tmp.path().join("other-data"), "s2".into());
    assert_eq!(other.rescan(&HashMap::new())[0].state, WidgetState::NeedsApproval);
    // The same instance after a restart remembers.
    let again = service(tmp.path());
    assert_eq!(again.rescan(&HashMap::new())[0].state, WidgetState::Approved);
}

#[test]
fn a_v1_module_entry_is_an_implied_trusted_package() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let hello = svc.widgets_dir.join("hello");
    std::fs::create_dir_all(&hello).unwrap();
    std::fs::write(hello.join("index.js"), "export default {}").unwrap();
    let mut entry = WidgetConfigType { label: "Hello".into(), module: "hello/index.js".into(), ..Default::default() };
    entry.block_def.meta.insert("view".into(), "ext:hello".into());
    let v1 = HashMap::from([("ext@hello".to_string(), entry)]);
    let p = svc.rescan(&v1)[0].clone();
    assert_eq!(p.id, "local.hello");
    assert_eq!(p.kind, WidgetKind::Trusted);
    assert!(p.implied);
    assert_eq!(p.state, WidgetState::NeedsApproval);
    assert_eq!(p.panes[0].view, "ext:hello");
    assert_eq!(p.panes[0].entry, "index.js");
    // A v1 widget keeps its own widget-bar entry; none is added.
    svc.approve("local.hello", &p.hash, "").unwrap();
    assert!(widget_entries(&svc.rescan(&v1)).is_empty());
}

#[test]
fn installing_counts_the_bytes_written_not_what_a_zip_declares() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("f");
    let mut near_full = Budget { bytes: MAX_PACKAGE_BYTES - 4, files: 0 };
    assert!(near_full.copy(&mut &b"1234"[..], &out).is_ok());
    assert!(near_full.copy(&mut &b"5"[..], &out).unwrap_err().contains("over 50 MB"));
    let mut many = Budget { bytes: 0, files: MAX_PACKAGE_FILES };
    assert!(many.copy(&mut &b"x"[..], &out).unwrap_err().contains("files"));
}

#[test]
fn install_copies_a_folder_or_a_zip_and_refuses_a_bad_one() {
    let tmp = tempfile::tempdir().unwrap();
    let widgets = tmp.path().join("widgets");
    let src = package(&tmp.path().join("src"), "acme.test");
    assert_eq!(install(&widgets, &src, false).unwrap(), "acme.test");
    assert!(widgets.join("acme.test/index.html").exists());
    assert!(install(&widgets, &src.join(MANIFEST_FILE), false).unwrap_err().contains("already installed"));
    assert_eq!(install(&widgets, &src.join(MANIFEST_FILE), true).unwrap(), "acme.test");
    // The replaced version's own files don't linger.
    std::fs::write(widgets.join("acme.test/old-only.txt"), "x").unwrap();
    assert_eq!(install(&widgets, &src, true).unwrap(), "acme.test");
    assert!(!widgets.join("acme.test/old-only.txt").exists());
    assert!(widgets.join("acme.test/index.html").exists());

    // A zip with the package in one top-level folder.
    let zip_path = tmp.path().join("pkg.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("acme.zipped/widget.json", opts).unwrap();
        std::io::Write::write_all(&mut z, manifest("acme.zipped").to_string().as_bytes()).unwrap();
        z.start_file("acme.zipped/index.html", opts).unwrap();
        std::io::Write::write_all(&mut z, b"<p>z</p>").unwrap();
        z.finish().unwrap();
    }
    assert_eq!(install(&widgets, &zip_path, false).unwrap(), "acme.zipped");

    let bad = tmp.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join(MANIFEST_FILE), "{\"manifestVersion\": 9}").unwrap();
    assert!(install(&widgets, &bad, false).is_err());
}
