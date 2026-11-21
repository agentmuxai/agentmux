// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the UI tools that talk to srv send and what they tell the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.
//! CaptureWindow and DiscoverWindows read the real desktop and are not run here.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path, and the body fields named here) and the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    path: &'static str,
    sent: Value,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, path, sent, says| Case { tool, args, reply, path, sent, says };
    vec![
        c(
            "UIScreenshot",
            json!({ "pane": "p1" }),
            json!({ "path": "/tmp/shot.png", "png_base64": "iVBORw0KGgo=" }),
            "/api/v1/ui/screenshot",
            json!({ "pane": "p1", "agent_id": "tester" }),
            // The stand-in's caller is in a container: its copy, not srv's path.
            &["Screenshot saved to ", "(inside your container) — use Read on that path"],
        ),
        c(
            "UIRecord",
            json!({ "op": "start", "pane": "p1", "fps": 5, "seconds": 9 }),
            json!({ "status": "started", "message": "Recording started." }),
            "/api/v1/ui/record",
            json!({ "op": "start", "pane": "p1", "fps": 5, "seconds": 9 }),
            &["Recording started."],
        ),
        c("UIClick", json!({ "selector": "#go" }), json!({}), "/api/v1/ui/click", json!({ "selector": "#go" }), &["Clicked \"#go\""]),
        c(
            "UIQuery",
            json!({ "selector": ".row", "limit": 3 }),
            json!({ "data": { "matches": [{ "tag": "div" }] } }),
            "/api/v1/ui/query",
            json!({ "selector": ".row", "limit": 3 }),
            &["\"tag\": \"div\""],
        ),
        c("UIQuery", json!({ "selector": ".row" }), json!({}), "/api/v1/ui/query", json!({}), &["[]"]),
        c(
            "ListShortcuts",
            json!({}),
            json!({ "ok": true, "data": { "shortcuts": [{ "keys": "Ctrl+K" }] } }),
            "/api/v1/ui/shortcuts/list",
            json!({ "agent_id": "tester" }),
            &["\"keys\": \"Ctrl+K\""],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:newtab", "target": " t1 " }),
            json!({ "ok": true, "data": { "ran": true } }),
            "/api/v1/ui/shortcuts/run",
            json!({ "command": "app:newtab", "target": "t1" }),
            &["Ran app:newtab."],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:quit" }),
            json!({ "ok": true, "data": { "pending": { "request_id": "r9" }, "reason": "the user must allow it" } }),
            "/api/v1/ui/shortcuts/run",
            json!({}),
            &["app:quit is pending: the user must allow it (request r9)."],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:nope" }),
            json!({ "ok": true, "data": { "ran": false, "reason": "unknown command" } }),
            "/api/v1/ui/shortcuts/run",
            json!({}),
            &["Didn't run app:nope: unknown command"],
        ),
        c(
            "PressKeys",
            json!({ "keys": "Ctrl+T" }),
            json!({ "ok": true, "data": { "pressed": true } }),
            "/api/v1/ui/shortcuts/press",
            json!({ "keys": "Ctrl+T" }),
            &["\"pressed\": true"],
        ),
        c(
            "SetShortcut",
            json!({ "command": "tab:new", "bindings": ["ctrl+shift+y"] }),
            json!({ "ok": true, "data": { "changed": true, "summary": "New tab: Ctrl+Shift+Y", "keys": ["ctrl+shift+y"], "note": "The person was told and can undo it from the notice." } }),
            "/api/v1/ui/shortcuts/set",
            json!({ "command": "tab:new", "bindings": ["ctrl+shift+y"] }),
            &["Changed: New tab: Ctrl+Shift+Y.", "can undo it"],
        ),
        c(
            "SetShortcut",
            json!({ "command": "tab:new", "bindings": [] }),
            json!({ "ok": true, "data": { "changed": false, "reason": "Ctrl+Shift+W is Close pane; an agent does not take a key from another command" } }),
            "/api/v1/ui/shortcuts/set",
            json!({ "command": "tab:new", "bindings": [] }),
            &["Not changed: Ctrl+Shift+W is Close pane"],
        ),
        c(
            "ResetShortcuts",
            json!({ "all": true }),
            json!({ "ok": true, "data": { "changed": true, "summary": "all shortcuts back on their defaults (2 commands)", "keys": [], "note": "The person was told." } }),
            "/api/v1/ui/shortcuts/reset",
            json!({ "all": true }),
            &["Changed: all shortcuts back on their defaults"],
        ),
    ]
}

#[tokio::test]
async fn every_ui_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = &seen[0];
        assert_eq!(path, case.path, "{} {}", case.tool, case.args);
        for (key, want) in case.sent.as_object().unwrap() {
            assert_eq!(body.get(key), Some(want), "{} {}: body {body}", case.tool, case.args);
        }
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("UIRecord", json!({}), "record needs op"),
        ("UIRecord", json!({ "op": "pause" }), "record needs op"),
        ("UIClick", json!({}), "missing required parameter: selector"),
        ("UIQuery", json!({}), "missing required parameter: selector"),
        ("RunCommand", json!({}), "missing required parameter: command"),
        ("PressKeys", json!({}), "missing required parameter: keys"),
    ];
    for (tool, args, wanted) in refusals {
        let srv = stub_srv("200 OK", json!({}));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
        assert!(srv.seen.lock().unwrap().is_empty(), "{tool} {args} asked srv");
    }
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_step_and_the_status() {
    let failures = [
        ("UIScreenshot", json!({}), "screenshot failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIRecord", json!({ "op": "stop" }), "record failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIClick", json!({ "selector": "#a" }), "click failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIQuery", json!({ "selector": "#a" }), "query failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ListShortcuts", json!({}), "shortcuts/list failed: HTTP 500 Internal Server Error — no error message"),
        ("RunCommand", json!({ "command": "c" }), "shortcuts/run failed: HTTP 500 Internal Server Error — no error message"),
        ("PressKeys", json!({ "keys": "k" }), "shortcuts/press failed: HTTP 500 Internal Server Error — no error message"),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_shortcut_answer_that_is_not_ok_is_a_failure_even_with_a_200() {
    let srv = stub_srv("200 OK", json!({ "ok": false, "error": "no window" }));
    let fx = Fixture::new(&srv);
    let err = call("ListShortcuts", &json!({}), &fx.cx()).await.unwrap_err().to_string();
    assert!(err.contains("shortcuts/list failed: HTTP 200 OK — no window"), "{err}");
}

#[tokio::test]
async fn a_name_that_is_not_a_ui_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("UINope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}


// A container agent cannot read srv's folders: the screenshot and recording it is told about are its own copies.

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nfake image bytes";

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn a_native_agent_is_told_the_hosts_screenshot_path_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let shot = UiScreenshotResponse { path: "/host/tmp/s.png".into(), png_base64: b64(PNG_BYTES) };
    let said = screenshot_text(&shot, false, dir.path()).unwrap();
    assert!(said.contains("Screenshot saved to /host/tmp/s.png — use Read on that path"), "{said}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "a native agent gets no copy");
}

#[test]
fn a_container_agents_screenshot_is_a_readable_file_in_its_own_folder() {
    let dir = tempfile::tempdir().unwrap();
    let shot = UiScreenshotResponse { path: "/host/tmp/s.png".into(), png_base64: b64(PNG_BYTES) };
    let said = screenshot_text(&shot, true, dir.path()).unwrap();
    assert!(!said.contains("/host/tmp"), "the host's path is not given: {said}");
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    let path = files[0].path();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
    assert!(said.contains(&path.to_string_lossy().to_string()), "{said}");
    assert!(said.contains("files") && said.contains("Tasks") && said.contains("Message"), "{said}");
    assert_eq!(std::fs::read(&path).unwrap(), PNG_BYTES);
    // The tools that take `files` accept it.
    let checked = super::super::send_files::check(&[path.to_string_lossy().into_owned()]).unwrap();
    assert_eq!((checked[0].size, checked[0].name.ends_with(".png")), (PNG_BYTES.len() as u64, true));
}

#[test]
fn a_container_screenshot_srv_sent_no_image_for_says_so_instead_of_naming_a_host_path() {
    let dir = tempfile::tempdir().unwrap();
    let shot = UiScreenshotResponse { path: "/host/tmp/s.png".into(), png_base64: String::new() };
    let err = screenshot_text(&shot, true, dir.path()).unwrap_err().to_string();
    assert!(err.contains("on the host") && !err.contains("/host/tmp"), "{err}");
    let bad = UiScreenshotResponse { path: "/h/s.png".into(), png_base64: "not base64!".into() };
    assert!(screenshot_text(&bad, true, dir.path()).unwrap_err().to_string().contains("could not be decoded"));
}

fn recording(path: Option<&str>, gif: Option<String>) -> UiRecordResponse {
    UiRecordResponse {
        status: "saved".into(),
        message: format!("Recording saved to {} (12 frames).", path.unwrap_or("-")),
        path: path.map(str::to_string),
        gif_base64: gif,
        frames: 12,
        skipped: 0,
        seconds: 2.0,
    }
}

#[test]
fn a_container_agents_recording_is_its_own_copy_and_the_message_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let rec = recording(Some("/host/tmp/r.gif"), Some(b64(b"GIF89a-bytes")));
    let said = record_text(&rec, true, dir.path()).unwrap();
    assert!(!said.contains("/host/tmp") && said.contains("(inside your container)") && said.contains("12 frames"), "{said}");
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap(), b"GIF89a-bytes");
    // Native, or a call that finished nothing (a start), is told what srv said.
    assert_eq!(record_text(&rec, false, dir.path()).unwrap(), rec.message);
    let started = recording(None, None);
    assert_eq!(record_text(&started, true, dir.path()).unwrap(), started.message);
    // A finished recording with no bytes cannot be offered.
    assert!(record_text(&recording(Some("/h/r.gif"), None), true, dir.path()).is_err());
}

#[test]
fn copies_an_hour_old_go_and_newer_ones_and_other_files_stay() {
    let dir = tempfile::tempdir().unwrap();
    let old_png = dir.path().join("old.png");
    let old_gif = dir.path().join("old.gif");
    let fresh = dir.path().join("fresh.png");
    let notes = dir.path().join("notes.txt");
    for p in [&old_png, &old_gif, &fresh, &notes] {
        std::fs::write(p, b"x").unwrap();
    }
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    for p in [&old_png, &old_gif, &notes] {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(two_hours_ago).unwrap();
    }
    prune_old_media(dir.path());
    assert!(!old_png.exists() && !old_gif.exists());
    assert!(fresh.exists() && notes.exists(), "a new copy and a file that is not a capture stay");
}

#[tokio::test]
async fn ui_record_asks_srv_for_the_file_in_a_container_and_names_the_copy() {
    let reply = json!({ "status": "saved", "message": "Recording saved to /host/r.gif (3 frames).", "path": "/host/r.gif", "gif_base64": b64(b"GIF89a") });
    let srv = stub_srv("200 OK", reply);
    let fx = Fixture::new(&srv);
    let said = call("UIRecord", &json!({ "op": "stop" }), &fx.cx()).await.unwrap();
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen[0].1.get("inline"), Some(&json!(true)), "{}", seen[0].1);
    assert!(!said.contains("/host/r.gif") && said.contains("(inside your container)"), "{said}");
}
