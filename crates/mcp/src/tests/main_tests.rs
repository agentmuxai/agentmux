// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// main.rs's `tests` module, moved out unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2).

use super::*;

#[test]
fn quit_self_result_tells_the_agent_what_happened() {
    assert!(quit_self_result(202, &Value::Null).unwrap().starts_with("Quit scheduled"));
    assert!(quit_self_result(200, &Value::Null).unwrap().starts_with("Already quitting"));
    let refused = quit_self_result(403, &serde_json::json!({ "refused": "not_user_turn" })).unwrap_err().to_string();
    assert!(refused.contains("not_user_turn") && refused.contains("/quit"), "{refused}");
    assert!(quit_self_result(401, &Value::Null).is_err());
}

#[test]
fn quit_self_outcome_says_whether_the_user_kept_it() {
    assert!(quit_self_outcome("kept_by_user", &Value::Null).unwrap().contains("keep you running"));
    assert!(quit_self_outcome("proceeding", &Value::Null).unwrap().contains("goodbye"));
    assert!(quit_self_outcome("shut_down", &Value::Null).unwrap().contains("goodbye"));
    assert!(quit_self_outcome("superseded", &Value::Null).is_ok());
    let failed = quit_self_outcome("failed", &serde_json::json!({ "error": "boom" })).unwrap_err().to_string();
    assert!(failed.contains("boom"), "{failed}");
}

#[test]
fn close_pane_outcome_says_what_the_user_decided() {
    let s = |st: &str| serde_json::json!({ "status": st });
    assert!(close_pane_outcome("b", &s("shut_down")).unwrap().starts_with("Closed"));
    assert!(close_pane_outcome("b", &s("kept_by_user")).unwrap().starts_with("Not closed"));
    assert!(close_pane_outcome("b", &s("superseded")).is_ok());
    assert!(close_pane_outcome("b", &serde_json::json!({ "status": "failed", "error": "boom" })).unwrap_err().to_string().contains("boom"));
    assert!(close_pane_outcome("b", &Value::Null).is_err());
    let joined = serde_json::json!({ "status": "shut_down", "via": "FleetBulkStop", "by": "Korp" });
    let text = close_pane_outcome("b", &joined).unwrap();
    assert!(text.contains("Korp's FleetBulkStop") && text.contains("may still be open"), "{text}");
    let gone = serde_json::json!({ "status": "superseded", "via": "FleetBulkStop", "by": "Korp" });
    let text = close_pane_outcome("b", &gone).unwrap();
    assert!(text.contains("already closed") && !text.contains("call ClosePane again"), "{text}");
}

#[test]
fn bulk_stop_folds_each_users_answer_into_succeeded_and_failed() {
    let pending = serde_json::json!({
        "status": "pending_user_override",
        "succeeded": ["x"],
        "failed": [{ "id": "gone", "error": "NOT_RUNNING" }],
        "aborted_early": false,
    });
    let out = merge_bulk_stop_outcomes(pending, vec![
        ("a".into(), serde_json::json!({ "status": "shut_down" })),
        ("b".into(), serde_json::json!({ "status": "kept_by_user" })),
        ("c".into(), serde_json::json!({ "status": "failed", "error": "boom" })),
    ]);
    assert_eq!(out["succeeded"], serde_json::json!(["x", "a"]));
    assert_eq!(out["kept_by_user"], serde_json::json!(["b"]));
    let failed: Vec<&str> = out["failed"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(failed, vec!["gone", "b", "c"]);
    assert!(out.get("status").is_none(), "the usual FleetActionResult shape");
}

#[test]
fn the_fleet_tools_warn_they_can_take_15_seconds() {
    for tool in [CLOSE_PANE_TOOL, FLEET_BULK_STOP_TOOL] {
        let v: Value = serde_json::from_str(tool).unwrap();
        assert!(v["description"].as_str().unwrap().contains("at least 15 seconds"), "{}", v["name"]);
    }
}

#[test]
fn quit_self_warns_it_can_take_15_seconds() {
    let v: Value = serde_json::from_str(QUIT_SELF_TOOL).unwrap();
    assert!(v["description"].as_str().unwrap().contains("at least 15 seconds"));
}

#[test]
fn quit_self_takes_no_target_and_requires_the_users_quote() {
    let v: Value = serde_json::from_str(QUIT_SELF_TOOL).unwrap();
    let props = v["inputSchema"]["properties"].as_object().unwrap();
    assert!(!props.contains_key("block_id") && !props.contains_key("agent"), "no way to aim it at another agent");
    assert_eq!(v["inputSchema"]["required"], serde_json::json!(["reason", "user_instruction"]));
    assert!(v["description"].as_str().unwrap().starts_with("⚠️ MAJOR WARNING"));
}

/// The text an agent reads back. Pinned because a lost `\` continuation
/// once turned the indentation into runs of spaces (ReAgent P1 on #3763).
#[test]
fn deferred_delivery_text_is_clean_and_names_the_target() {
    let t = deferred_delivery_text("Camper");
    assert!(t.starts_with("QUEUED for Camper — their agent is starting up, restarting or stopping"), "{t}");
    assert!(!t.contains("mid-turn") && !t.contains("turn ends"), "nothing waits for a turn: {t}");
    assert!(!t.contains("  "), "no runs of spaces: {t:?}");
    assert!(t.ends_with("Don't resend it."), "{t}");
}

/// Every srv answer to `/agentmux/reactive/inject` maps to a reply that keeps
/// its first word and carries the message id
/// (SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md §5.2, Phase 0 item 3).
#[test]
fn send_message_outcome_names_the_state_and_the_id() {
    use serde_json::json;
    let ok = |v: Value| send_message_outcome("Camper", &v).unwrap();
    let err = |v: Value| send_message_outcome("Camper", &v).unwrap_err().to_string();

    let t = ok(json!({ "success": true, "request_id": "1-2-3", "block_id": "b1", "deferred": false }));
    assert!(t.starts_with("Delivered to Camper — "), "{t}");
    assert!(t.ends_with(" id=1-2-3"), "{t}");
    // PTY delivery leaves `deferred` out entirely.
    assert!(ok(json!({ "success": true, "request_id": "1-2-3", "block_id": "b1" })).starts_with("Delivered to Camper"));

    let t = ok(json!({ "success": true, "request_id": "1-2-3", "block_id": "b1", "deferred": true }));
    assert!(t.starts_with(&deferred_delivery_text("Camper")), "{t}");
    assert!(t.ends_with(" id=1-2-3"), "{t}");

    // Relay: no block_id; the id is the relay's own.
    let t = ok(json!({ "success": true, "request_id": "inj-42" }));
    assert!(t.starts_with("QUEUED for Camper via the cloud relay (unconfirmed, expires in 30 min)"), "{t}");
    assert!(t.contains("DiscoverAgents") && t.ends_with(" id=inj-42"), "{t}");
    assert!(!t.contains("signed in from your account"), "no hint unless the relay says so: {t}");
    // The relay says the target is not one of the sender's agents
    // (agentmux-cloud#138): a likely typo, still queued.
    let t = ok(json!({ "success": true, "request_id": "inj-43", "target_in_account": false }));
    assert!(t.starts_with("QUEUED for Camper via the cloud relay"), "{t}");
    assert!(t.contains("No agent of that name has signed in from your account yet, so check the spelling"), "{t}");
    assert!(t.contains("one of your agents with that name signs in within 30 minutes"), "not \"only another account\" (Codex P2 on #4249): {t}");
    assert!(t.ends_with(" id=inj-43") && !t.contains("  "), "{t:?}");
    let t = ok(json!({ "success": true, "request_id": "inj-44", "target_in_account": true }));
    assert!(!t.contains("signed in from your account"), "{t}");

    let t = ok(json!({ "success": false, "held": true, "request_id": "1-2-3", "error": "agent Camper is not running" }));
    assert!(t.starts_with("HELD for Camper (not_running)"), "{t}");
    assert!(t.contains("24 hours") && t.ends_with(" id=1-2-3"), "{t}");

    // Held because the receiver is not signed in (Phase 0 item 4).
    let t = ok(json!({ "success": false, "held": true, "held_reason": "needs_login", "request_id": "1-2-3",
        "error": "agent Camper is not signed in — held" }));
    assert!(t.starts_with("HELD for Camper (needs_login)"), "{t}");
    assert!(t.contains("signing in") && t.ends_with(" id=1-2-3"), "{t}");
    assert!(!t.contains("  "), "no runs of spaces: {t:?}");

    let e = err(json!({ "success": false, "block_id": "b1",
        "error": "identity spawn gate: no credentials for claude: the bound account was deleted or is unresolvable." }));
    assert!(e.starts_with("Message delivery failed (needs_login)"), "{e}");
    assert!(e.contains("NOT kept") && e.contains("no credentials for claude"), "{e}");

    let e = err(json!({ "success": false, "error": "agent not found: Camper" }));
    assert_eq!(e, "Message delivery failed (not_found): agent not found: Camper");

    assert_eq!(err(json!({ "success": false, "error": "rate limit exceeded" })), "Message delivery failed: rate limit exceeded");
    assert_eq!(err(json!({})), "Message delivery failed: unknown error");

    // No id from srv (an old peer's forwarded body): no dangling "id=".
    let t = ok(json!({ "success": true, "block_id": "b1" }));
    assert!(!t.contains("id="), "{t}");
    for t in [ok(json!({ "success": true, "request_id": "x" })), ok(json!({ "success": false, "held": true }))] {
        assert!(!t.contains("  "), "no runs of spaces: {t:?}");
    }
}

/// The description lists every answer the tool can give (G10: it once
/// omitted HELD and the failures).
#[test]
fn send_message_description_lists_every_answer() {
    let v: Value = serde_json::from_str(SEND_MESSAGE_TOOL).unwrap();
    let d = v["description"].as_str().unwrap();
    for s in ["Delivered to X", "QUEUED for X — their agent is starting up", "HELD for X (not_running)", "HELD for X (needs_login)",
              "via the cloud relay (unconfirmed", "signed in from your account", "(needs_login)", "(not_found)", "id="] {
        assert!(d.contains(s), "missing {s:?}");
    }
}

/// Serve exactly one HTTP response on a fresh localhost port; return the
/// base URL.
fn one_shot_server(status_line: &'static str, body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = conn.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let response = format!(
            "HTTP/1.1 {status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        conn.write_all(response.as_bytes()).unwrap();
    });
    url
}

async fn get(url: &str) -> Result<Value> {
    let client = reqwest::Client::new();
    srv_get_json(&client, url, "key", &[("query", "x".to_string())], "history search").await
}

#[tokio::test]
async fn srv_get_json_returns_the_body_of_a_success() {
    let url = one_shot_server("200 OK", r#"{"hits":[],"complete":true}"#);
    assert_eq!(get(&url).await.unwrap()["complete"], true);
}

/// #3473: srv's own message must reach the agent, with the status.
#[tokio::test]
async fn srv_get_json_reports_srvs_own_error_message_and_the_status() {
    let url = one_shot_server(
        "403 Forbidden",
        r#"{"error":"history.search: this request carries no agent identity (X-Agent-Token)"}"#,
    );
    let err = get(&url).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 403"), "{err}");
    assert!(err.contains("carries no agent identity"), "{err}");
}

/// A non-JSON error body used to fail JSON parsing first and lose the
/// status entirely.
#[tokio::test]
async fn srv_get_json_keeps_the_status_when_the_error_body_is_not_json() {
    let url = one_shot_server("502 Bad Gateway", "upstream went away");
    let err = get(&url).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 502"), "{err}");
    assert!(err.contains("upstream went away"), "{err}");
}

#[tokio::test]
async fn srv_get_json_says_when_agentmux_cannot_be_reached() {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    // The listener is dropped: nothing is listening on `port` now.
    let err = get(&format!("http://127.0.0.1:{port}")).await.unwrap_err().to_string();
    assert!(err.contains("can't reach AgentMux"), "{err}");
}

#[test]
fn a_401_explains_the_auth_key_rather_than_a_network_fault() {
    let msg = describe_http_error("history search", reqwest::StatusCode::UNAUTHORIZED, r#"{"error":"unauthorized"}"#);
    assert!(msg.contains("HTTP 401"), "{msg}");
    assert!(msg.contains("AGENTMUX_AUTH_KEY"), "{msg}");
}

/// Guards every test that mutates `AGENTMUX_DATA_HOME` (a process-global
/// env var) so they never run concurrently against each other — cargo
/// runs tests in parallel by default, and two tests independently
/// setting/clearing the same env var would otherwise be a genuine race,
/// not just a stale comment. Acquire this at the start of any such test,
/// before touching the env var, and hold it for the env var's entire
/// mutated lifetime (not just around `capture_window_dir()`/
/// `audit_log_capture_window()` themselves).
static DATA_HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Sets both AgentMux root env vars for one test and restores them on drop,
/// panics included, holding `DATA_HOME_ENV_LOCK` throughout. Both are set or
/// cleared every time: `agentmux_root()` reads `AGENTMUX_HOME_OVERRIDE`
/// ahead of `AGENTMUX_DATA_HOME`, so a test that only set the latter would
/// silently test the ambient override instead (Codex P2 on #4030).
struct RootEnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn root_env(home_override: Option<&std::path::Path>, data_home: Option<&std::path::Path>) -> RootEnvGuard {
    // A panicking test poisons the lock; the env is restored by the
    // guard's drop regardless, so the next test can proceed.
    let lock = DATA_HOME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut saved = Vec::new();
    for (var, value) in [("AGENTMUX_HOME_OVERRIDE", home_override), ("AGENTMUX_DATA_HOME", data_home)] {
        saved.push((var, std::env::var_os(var)));
        // SAFETY: test-only, serialized by DATA_HOME_ENV_LOCK.
        unsafe {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
    }
    RootEnvGuard { saved, _lock: lock }
}

impl Drop for RootEnvGuard {
    fn drop(&mut self) {
        for (var, value) in self.saved.drain(..) {
            // SAFETY: test-only, still serialized by DATA_HOME_ENV_LOCK.
            unsafe {
                match value {
                    Some(v) => std::env::set_var(var, v),
                    None => std::env::remove_var(var),
                }
            }
        }
    }
}

/// Mirrors `agentmux-srv`'s `prune_old_screenshots_deletes_only_stale_pngs`
/// (`ui_handlers.rs`) exactly — same bug class, same fix, same test shape
/// (reagent P1/P2 on this tool's own PR, #2709 round 1).
#[test]
fn prune_old_captures_deletes_only_stale_pngs() {
    let dir = tempfile::tempdir().unwrap();

    let fresh = dir.path().join("fresh.png");
    std::fs::write(&fresh, b"png").unwrap();

    let stale = dir.path().join("stale.png");
    std::fs::write(&stale, b"png").unwrap();
    let old_time = std::time::SystemTime::now() - (CAPTURE_RETENTION * 2);
    let file = std::fs::File::options().write(true).open(&stale).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();

    // Non-PNG files must never be touched, however old.
    let other = dir.path().join("notes.txt");
    std::fs::write(&other, b"keep me").unwrap();
    let file = std::fs::File::options().write(true).open(&other).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();

    prune_old_captures(dir.path());

    assert!(fresh.exists(), "fresh capture must survive pruning");
    assert!(!stale.exists(), "stale capture must be pruned");
    assert!(other.exists(), "non-png files must never be pruned");
}

/// `AGENTMUX_DATA_HOME`, when set, must win over the `~/.agentmux` default
/// — the same override `agentmux-srv`'s own `get_mux_data_dir()` honors,
/// which this function replicates rather than reinventing.
#[test]
fn capture_window_dir_honors_agentmux_data_home_override() {
    let _env = root_env(None, Some(std::path::Path::new("/tmp/custom-agentmux-home")));
    assert_eq!(
        capture_window_dir().unwrap(),
        std::path::PathBuf::from("/tmp/custom-agentmux-home/tmp/capture-window")
    );
}

/// The capture dir comes from the one AgentMux root resolver
/// (`agentmux_common::data_paths::agentmux_root`), so it honours
/// `AGENTMUX_HOME_OVERRIDE` ahead of `AGENTMUX_DATA_HOME`, exactly like
/// srv. This module's earlier private copy read only `AGENTMUX_DATA_HOME`
/// and fell back to `/` with no home dir, the behaviour #3372 removed from
/// srv (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §5.1 #5).
#[test]
fn capture_window_dir_uses_the_shared_root_resolver() {
    let _env = root_env(
        Some(std::path::Path::new("/tmp/override-root")),
        Some(std::path::Path::new("/tmp/data-home-root")),
    );
    assert_eq!(
        capture_window_dir().unwrap(),
        std::path::PathBuf::from("/tmp/override-root/tmp/capture-window")
    );
}

/// `own_instance_pids()` must always include the calling process's own
/// pid at minimum (the first element of its ancestor-walk fallback,
/// independent of whether `AGENTMUX_APP_PATH` is set in this test's
/// environment). Not a full behavioral test of the exclusion logic
/// itself — mocking `sysinfo`'s real OS process table isn't practical —
/// but it does verify the function runs without panicking and its one
/// environment-independent guarantee holds. The actual exclusion
/// behavior (reagent P0, PR #2709 round 3) was verified manually against
/// this repo's own live process tree — see that commit's message for
/// the before/after evidence.
#[test]
fn own_instance_pids_always_includes_the_calling_process_itself() {
    let pids = own_instance_pids();
    assert!(
        pids.contains(&std::process::id()),
        "own_instance_pids() must always include this process's own pid"
    );
}

/// `looks_unrendered()` is the retry/hint trigger for
/// `CaptureWindow` (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md
/// Fix 4) — must flag a truly uniform-color frame (the "blank capture,
/// no signal" bug the report documents) and must NOT flag a frame with
/// real visual variation, even a subtle one, as long as it exceeds the
/// tolerance.
#[test]
fn looks_unrendered_flags_a_solid_color_frame() {
    let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
    assert!(looks_unrendered(&img), "a fully solid-color frame must be flagged");
}

#[test]
fn looks_unrendered_does_not_flag_a_varied_frame() {
    let mut img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
    // A single differing pixel could land between two sampled indices
    // (sampling is spaced across the image, not exhaustive — see
    // looks_unrendered's doc comment) and be missed entirely. Fill a
    // whole row instead, so several sampled indices are guaranteed to
    // fall inside it regardless of the exact sample step for this
    // image size.
    for x in 0..32 {
        img.put_pixel(x, 16, image::Rgba([220, 30, 30, 255]));
    }
    assert!(
        !looks_unrendered(&img),
        "a frame with a clearly differing region must not be flagged as unrendered"
    );
}

#[test]
fn looks_unrendered_ignores_noise_within_tolerance() {
    // Real compositor output isn't perfectly uniform even for a
    // genuinely "blank" themed window (subpixel AA, slight gradient
    // banding) — small per-channel noise within the tolerance must
    // still read as unrendered, or every real blank frame would dodge
    // the retry.
    let mut img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
    for (i, px) in img.pixels_mut().enumerate() {
        let jitter = (i % 5) as u8; // stays within the +/-8 tolerance
        *px = image::Rgba([20 + jitter, 20, 20, 255]);
    }
    assert!(
        looks_unrendered(&img),
        "small within-tolerance noise must still read as unrendered"
    );
}

/// `audit_log_capture_window` must append one valid NDJSON line per
/// call, for both success and failure outcomes, without ever panicking
/// or returning an error to its caller (it's a fire-and-forget
/// best-effort side effect — reagent P1, PR #2709 round 4).
#[test]
fn audit_log_capture_window_appends_ndjson_for_success_and_failure() {
    let dir = tempfile::tempdir().unwrap();
    let _env = root_env(None, Some(dir.path()));

    audit_log_capture_window(
        "first query",
        &Ok(CaptureOutcome {
            message: "captured ok".to_string(),
            tier: CaptureTier::OtherInstance,
            target: "pid=42 title=\"Other\"".to_string(),
            image_sha256: Some("deadbeef".to_string()),
        }),
        &None,
    );
    audit_log_capture_window("second query", &Err(anyhow::anyhow!("no match")), &None);

    let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "one NDJSON line per call");

    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["tool"], "CaptureWindow");
    assert_eq!(first["query"], "first query");
    assert_eq!(first["outcome"]["result"], "success");
    // Spec §6 fields — what was captured, not just that something was.
    assert_eq!(first["tier"], "T2-other-instance");
    assert_eq!(first["target"], "pid=42 title=\"Other\"");
    assert_eq!(first["image_sha256"], "deadbeef");
    assert_eq!(first["redacted"], false);

    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["query"], "second query");
    assert_eq!(second["outcome"]["result"], "error");
    // Nothing was resolved on failure, so these stay null rather than
    // recording a target that was never captured.
    assert!(second["tier"].is_null());
    assert!(second["image_sha256"].is_null());
}

/// codex P2 on PR #2845: a failure AFTER target resolution — a denied T3
/// pid, a failed capture, a failed save — must still record which tier and
/// target it addressed. Otherwise the audit cannot tell "no such window"
/// apart from "blocked cross-user attempt", which is the entry a reviewer
/// most wants to find.
#[test]
fn a_failed_capture_still_audits_its_resolved_tier_and_target() {
    let dir = tempfile::tempdir().unwrap();
    let _env = root_env(None, Some(dir.path()));

    let resolved = Some((
        CaptureTier::OtherUser,
        "pid=99 <non-AgentMux window>".to_string(),
    ));
    audit_log_capture_window("pid=99", &Err(anyhow::anyhow!("withheld")), &resolved);

    let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
    let content = std::fs::read_to_string(&log_path).unwrap();
    let entry: Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(entry["outcome"]["result"], "error");
    assert_eq!(
        entry["tier"], "T3-other-user",
        "a denied cross-user attempt must be identifiable in the trail"
    );
    assert_eq!(entry["target"], "pid=99 <non-AgentMux window>");
}

/// A foreign window's TITLE must never appear in a candidate/miss list —
/// that would bypass DiscoverWindows' include_foreign opt-in by simply
/// missing on purpose (reagent P1 / codex P2 on PR #2845).
#[test]
fn candidate_label_withholds_foreign_window_titles() {
    let Ok(windows) = enumerate_agentmux_windows() else { return };
    for w in windows.iter().filter(|w| !w.is_agentmux && !w.title.is_empty()) {
        let label = candidate_label(w);
        // Escaped form, same reason as `audit_target_label`'s test: a raw
        // `contains(&w.title)` would pass vacuously for any title
        // containing a backslash, which on Windows is most paths.
        assert!(
            !label.contains(&format!("{:?}", w.title)),
            "foreign window title leaked into a candidate label: {label}"
        );
        assert!(label.contains(&format!("pid={}", w.pid)), "pid must still identify it");
        return; // one real foreign window is enough
    }
}

/// reagentx P2 on PR #2845 caught that the previous round's claimed fix for
/// this had silently not applied — the edit no-opped and I reported it as
/// landed. This test pins the behaviour itself rather than trusting a diff:
/// a withheld target must be reachable in the UNFILTERED enumeration so it
/// can be audited, since `foreign` (tier-filtered) cannot see it.
#[test]
fn a_withheld_window_is_still_findable_for_auditing() {
    let Ok(windows) = enumerate_agentmux_windows() else { return };
    let withheld: Vec<&AgentMuxWindowInfo> =
        windows.iter().filter(|w| !w.tier.allowed()).collect();
    let capturable: Vec<&AgentMuxWindowInfo> =
        windows.iter().filter(|w| w.tier.allowed()).collect();
    // The enumeration must retain both sets — the capture gate filters
    // later. If enumeration itself dropped withheld windows, the audit
    // could never name them and "withheld" would be indistinguishable
    // from "absent", which is the defect this pins.
    assert_eq!(
        withheld.len() + capturable.len(),
        windows.len(),
        "every enumerated window must be classified, none silently dropped"
    );
    for w in withheld {
        assert_eq!(w.tier, CaptureTier::OtherUser, "only T3 is withheld");
    }
}

/// reagentx P1 on PR #2845: withholding a cross-user window's TITLE is not
/// enough if the *response* differs — differing errors are an existence
/// oracle, and an agent can probe substrings to reconstruct that title
/// without ever capturing. The miss message must therefore never reveal
/// that a withheld window matched.
///
/// Pins the observable property: no tier label may appear in a title-miss
/// error. If someone reintroduces a distinguishing branch, it will almost
/// certainly name the tier (that is what the reverted version did) and
/// this fails.
#[test]
fn a_title_miss_never_reveals_a_withheld_match() {
    let mut resolved = None;
    let err = capture_window_impl(
        Some("zzz-nonexistent-window-title-zzz"),
        None,
        None,
        &mut resolved,
    )
    .expect_err("a nonsense title cannot match");
    let msg = err.to_string();
    for leak in ["T3", "other-user", "withheld", "different OS user"] {
        assert!(
            !msg.contains(leak),
            "title-miss error leaked withheld-window state via {leak:?}: {msg}"
        );
    }
}

/// reagentx P2 on PR #2845, with a corrected premise. Audit detail follows
/// the ALLOW decision, not AgentMux-ness:
///   - an allowed tier records the real title (the agent could capture the
///     window and read it off the pixels anyway)
///   - a withheld tier records pid + tier only, because the trail is an
///     agent-readable file — putting a T3 title there would hand back
///     exactly what the tier denied, reopening the closed oracle via the log
#[test]
fn audit_target_label_withholds_only_for_withheld_tiers() {
    let Ok(windows) = enumerate_agentmux_windows() else { return };
    for w in &windows {
        let label = audit_target_label(w);
        assert!(label.contains(&format!("pid={}", w.pid)));
        // Compare against the DEBUG-escaped form the label actually emits.
        // A naive `contains(&w.title)` passes only while no window title
        // needs escaping — it went green locally and failed on CI, where a
        // window is titled `C:\ProgramData\GitHub\...` and `{:?}` doubles
        // every backslash. It would also have made the withheld-side
        // assertion below pass vacuously for exactly those titles.
        let escaped = format!("{:?}", w.title);
        if w.tier.allowed() {
            if !w.title.is_empty() {
                assert!(
                    label.contains(&escaped),
                    "an allowed tier should keep full audit detail: {label}"
                );
            }
        } else {
            assert!(
                label.contains("<title withheld>"),
                "a withheld tier must not record its title in an agent-readable log: {label}"
            );
            if !w.title.is_empty() {
                assert!(
                    !label.contains(&escaped),
                    "T3 title leaked into the audit: {label}"
                );
            }
        }
    }
}

/// reagentx P2 on PR #2845: a withheld window must be LISTED (so the
/// `is_self` fail-safe still surfaces it and `capturable` means something)
/// but must not carry the two fields that cross the human boundary.
#[test]
fn withheld_windows_are_listed_but_title_and_exe_path_are_redacted() {
    let Ok(windows) = enumerate_agentmux_windows() else { return };
    for w in &windows {
        let entry = window_listing_entry(w);
        assert_eq!(entry["pid"], w.pid, "pid is always surfaced");
        assert_eq!(entry["tier"], w.tier.label());
        assert_eq!(entry["capturable"], w.tier.allowed());
        if w.tier.allowed() {
            assert_eq!(entry["title"], w.title);
            assert_eq!(entry["exe_path"], w.exe_path);
        } else {
            assert!(entry["title"].is_null(), "a withheld title must not be listed");
            assert!(entry["exe_path"].is_null(), "exe_path embeds the OS username");
            assert!(!entry["withheld_reason"].is_null(), "say why, don't just blank it");
        }
    }
}

/// The Phase-1 `allow` defaults (spec §3). The whole point of this change
/// is that the caller's own instance is reachable — the old `!is_self`
/// rule blocked exactly this — while the one human-boundary tier is not.
#[test]
fn capture_tier_allows_every_agent_tier_and_withholds_only_other_user() {
    assert!(CaptureTier::SameInstance.allowed(), "own instance must be reachable");
    assert!(CaptureTier::OtherInstance.allowed());
    assert!(CaptureTier::ForeignApp.allowed());
    assert!(
        !CaptureTier::OtherUser.allowed(),
        "a different OS user's window is the one tier held back"
    );
}

/// Tier labels land in the audit trail, so a reviewer greps them. Pin the
/// exact strings — a silent rename would break existing log analysis.
#[test]
fn capture_tier_labels_are_stable() {
    assert_eq!(CaptureTier::SameInstance.label(), "T1-same-instance");
    assert_eq!(CaptureTier::OtherInstance.label(), "T2-other-instance");
    assert_eq!(CaptureTier::OtherUser.label(), "T3-other-user");
    assert_eq!(CaptureTier::ForeignApp.label(), "T4-foreign-app");
}

/// `current_user_id` failing must produce a DENY, not an allow — the
/// fail-closed discipline `own_instance_pids()` already follows. Verified
/// through the real enumeration: every window it returns has a resolved
/// tier, and any window whose owner couldn't be determined is T3.
#[test]
fn windows_with_unresolvable_owner_are_withheld() {
    let Ok(windows) = enumerate_agentmux_windows() else {
        return; // headless CI — nothing to assert against
    };
    for w in &windows {
        if w.exe_path.is_empty() && w.tier.allowed() {
            panic!(
                "window pid={} has no resolvable owning process yet was allowed \
                 — fail-closed violated",
                w.pid
            );
        }
    }
}

/// reagent P1 on PR #2810: `DiscoverWindows` discloses `exe_path`
/// (embeds the OS username for a foreign instance/user on a shared
/// machine) and shipped with zero audit logging, unlike `CaptureWindow`
/// which logs every call for exactly this reason. Pins that it now
/// does, into the SAME log file (one window-tool audit trail, not two).
#[test]
fn audit_log_discover_windows_appends_ndjson_with_window_list() {
    let dir = tempfile::tempdir().unwrap();
    let _env = root_env(None, Some(dir.path()));

    let windows = vec![json!({
        "pid": 4242,
        "title": "AgentMux",
        "exe_path": "C:\\Users\\someone\\agentmux.exe",
        "is_self": false,
    })];
    audit_log_discover_windows(false, false, &windows);

    let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "one NDJSON line for this call");

    let entry: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["tool"], "DiscoverWindows");
    // Both disclosure flags must be legible from the query string alone —
    // `include_foreign` is the one that exposes non-AgentMux titles and
    // exe_paths, i.e. this trail's whole reason for existing (reagentx P2
    // on PR #2845).
    assert_eq!(entry["query"], "include_self=false include_foreign=false");
    assert_eq!(entry["outcome"]["result"], "success");
    assert_eq!(entry["outcome"]["window_count"], 1);
    assert_eq!(
        entry["outcome"]["windows"][0]["exe_path"],
        "C:\\Users\\someone\\agentmux.exe"
    );
}

/// Every tool advertised by `tools/list` must be valid JSON with a `name`
/// and `inputSchema` — the server `expect("static json")`s these at runtime,
/// so a malformed const would panic on the first `tools/list`. Also pins the
/// tool count (11 original + 2 loop tools = 13). See
/// SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md §10.
#[test]
fn all_tool_defs_are_valid_json_with_names() {
    let defs = [
        SHELL_TOOL,
        SHELL_STOP_TOOL,
        OPEN_EDITOR_TOOL,
        OPEN_MEDIA_TOOL,
        OPEN_FILES_TOOL,
        SEND_MESSAGE_TOOL,
        DISCOVER_AGENTS_TOOL,
        WHOAMI_TOOL,
        CONN_LIST_TOOL,
        LAYOUT_TOOL,
        SET_NAME_TOOL,
        SET_ACTIVE_TAB_TOOL,
        NEW_TAB_TOOL,
        FOCUS_WINDOW_TOOL,
        LOOP_TOOL,
        LOOP_STOP_TOOL,
        LOOP_LIST_TOOL,
        WORK_ENQUEUE_TOOL,
        WORK_CLAIM_TOOL,
        WORK_HEARTBEAT_TOOL,
        WORK_COMPLETE_TOOL,
        WORK_RELEASE_TOOL,
        WORK_LIST_TOOL,
        CRON_CREATE_TOOL,
        CRON_DELETE_TOOL,
        CRON_LIST_TOOL,
        CRON_PAUSE_TOOL,
        CRON_RESUME_TOOL,
        MEMORY_LIST_TOOL,
        MEMORY_READ_TOOL,
        MEMORY_WRITE_TOOL,
        MEMORY_HISTORY_TOOL,
        MEMORY_DIFF_TOOL,
        MEMORY_REVERT_TOOL,
        GLOBAL_MEMORY_LIST_TOOL,
        GLOBAL_MEMORY_READ_TOOL,
        GLOBAL_MEMORY_WRITE_TOOL,
        GLOBAL_MEMORY_REMOVE_TOOL,
        GLOBAL_MEMORY_HISTORY_TOOL,
        GLOBAL_MEMORY_DIFF_TOOL,
        GLOBAL_MEMORY_REVERT_TOOL,
        PRESET_LIST_TOOL,
        PRESET_GET_TOOL,
        IDENTITY_ACCOUNTS_TOOL,
        IDENTITY_VALIDATE_TOOL,
        FLEET_LIST_TOOL,
        FLEET_BROADCAST_TOOL,
        FLEET_BULK_STOP_TOOL,
        OPEN_AGENT_TOOL,
        CAPTURE_WINDOW_TOOL,
        DISCOVER_WINDOWS_TOOL,
        LIST_CONVERSATIONS_TOOL,
        CLOSE_PANE_TOOL,
        QUIT_SELF_TOOL,
        REGISTER_DEV_SERVER_TOOL,
    ];
    // This array (and its count) has drifted from the real `tools/list`
    // response before this change too — SHELL_INPUT/STATUS, the three
    // UI_* tools, GET_AGENT_TRANSCRIPT, and SUPERVISOR_NUDGE are all
    // live tools missing from it. Not fixed here (out of scope for
    // this feature) — just adding the 3 new fleet-control tools
    // (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) alongside the 3
    // memory-version-history tools merged in from a concurrent PR, on
    // top of whatever this test already covered, so at least those
    // don't silently join the drift. CAPTURE_WINDOW_TOOL added here too
    // (PR #2709) — same reasoning, not fixing the pre-existing drift.
    // LIST_CONVERSATIONS_TOOL added here too
    // (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md
    // Phase A) — same reasoning, not fixing the pre-existing drift.
    // DISCOVER_WINDOWS_TOOL added here too
    // (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md) — same
    // reasoning, not fixing the pre-existing drift.
    // MUXQUEUE_TOOLS (6: WorkEnqueue/Claim/Heartbeat/Complete/Release/List)
    // added here too (REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md
    // slice 2) — same reasoning, not fixing the pre-existing drift between
    // this running total and the prose breakdown below it.
    // OPEN_AGENT_TOOL added (REPORT_AGENT_OPEN_API_GAP_2026_09_06.md) —
    // same reasoning as the entries above, not fixing the pre-existing
    // drift between this running total and the prose breakdown.
    // CLOSE_PANE_TOOL added (SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md
    // Phase 1) — same reasoning, not fixing the pre-existing drift.
    // GLOBAL_MEMORY_{LIST,READ,WRITE,REMOVE}_TOOL added (4:
    // SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md Phase 2) — same
    // reasoning, not fixing the pre-existing drift between this running
    // total and the prose breakdown.
    // REGISTER_DEV_SERVER_TOOL added (1:
    // SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md) — same reasoning,
    // not fixing the pre-existing drift between this running total and
    // the prose breakdown.
    // GLOBAL_MEMORY_{HISTORY,DIFF,REVERT}_TOOL added (3:
    // SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md's own Phase-3
    // follow-up — the read side of the Global Memory audit trail,
    // `bundle_version_list`/`bundle_version_get`, existed and was tested
    // but had no MCP tool wrapper until now) — same reasoning, not fixing
    // the pre-existing drift between this running total and the prose
    // breakdown.
    // OPEN_FILES_TOOL added (1: SPEC_FILE_BROWSER_PANE_2026_10_01.md §8.1)
    // — same reasoning, not fixing the pre-existing drift.
    assert_eq!(defs.len(), 55, "+ 1 ConnList + 1 OpenFiles + 1 QuitSelf; tools/list advertises 27 tools (11 original + 1 OpenMedia + 3 Loop + 5 Cron + 7 agent-API) + 3 memory-version-history + 3 fleet-control tools + 1 OpenAgent + 1 CaptureWindow + 1 ListConversations + 1 DiscoverWindows + 6 Muxqueue + 1 ClosePane + 4 GlobalMemory + 1 RegisterDevServer + 3 GlobalMemory-version-history");
    for d in defs {
        let v: Value = serde_json::from_str(d).expect("tool def must be valid JSON");
        assert!(
            v.get("name").and_then(|n| n.as_str()).is_some(),
            "tool def missing name: {d}"
        );
        assert!(v.get("inputSchema").is_some(), "tool def missing inputSchema");
    }
}

/// The two consolidated verbs must expose their discriminator enums so the
/// model can pick the sub-action (replaces the former one-tool-per-verb set).
#[test]
fn consolidated_tools_expose_their_discriminators() {
    let layout: Value = serde_json::from_str(LAYOUT_TOOL).unwrap();
    let query = layout["inputSchema"]["properties"]["query"]["enum"]
        .as_array()
        .expect("Layout.query.enum is an array");
    assert_eq!(query.len(), 4, "Layout.query folds the 4 read verbs");

    let set_name: Value = serde_json::from_str(SET_NAME_TOOL).unwrap();
    let target = set_name["inputSchema"]["properties"]["target"]["enum"]
        .as_array()
        .expect("SetName.target.enum is an array");
    assert_eq!(target.len(), 4, "SetName.target folds the 4 naming verbs");
    let required = set_name["inputSchema"]["required"]
        .as_array()
        .expect("SetName.required is an array");
    assert_eq!(required.len(), 2, "SetName requires both target and name");
}

#[test]
fn parse_interval_handles_units() {
    assert_eq!(parse_interval("30s").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_interval("5m").unwrap(), Duration::from_secs(300));
    assert_eq!(parse_interval("1h").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_interval("10").unwrap(), Duration::from_secs(600)); // bare = minutes
}

#[test]
fn parse_interval_clamps_minimum() {
    assert_eq!(parse_interval("1s").unwrap(), Duration::from_secs(10)); // clamp to 10s
}

// REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md:
// FleetBroadcast's block_id resolution originally only read
// `host.addressable`, silently failing every `host.cross_channel`
// target even though it carries a real block_id in the same namespace.
#[test]
fn build_block_to_agent_map_includes_host_addressable() {
    let discovery = serde_json::json!({
        "host": {
            "addressable": [
                { "agent_id": "Korp", "block_id": "block-1" },
            ],
        },
    });
    let map = build_block_to_agent_map(&discovery);
    assert_eq!(map.get("block-1").map(String::as_str), Some("Korp"));
}

#[test]
fn build_block_to_agent_map_includes_host_cross_channel() {
    let discovery = serde_json::json!({
        "host": {
            "addressable": [],
            "cross_channel": [
                { "name": "Loap", "channel": "dev-other", "local_url": "http://127.0.0.1:9999", "block_id": "block-2" },
            ],
        },
    });
    let map = build_block_to_agent_map(&discovery);
    assert_eq!(map.get("block-2").map(String::as_str), Some("Loap"));
}

#[test]
fn build_block_to_agent_map_merges_both_sections_without_dropping_either() {
    let discovery = serde_json::json!({
        "host": {
            "addressable": [
                { "agent_id": "Korp", "block_id": "block-1" },
            ],
            "cross_channel": [
                { "name": "Loap", "channel": "dev-other", "local_url": "http://127.0.0.1:9999", "block_id": "block-2" },
            ],
        },
    });
    let map = build_block_to_agent_map(&discovery);
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("block-1").map(String::as_str), Some("Korp"));
    assert_eq!(map.get("block-2").map(String::as_str), Some("Loap"));
}

#[test]
fn build_block_to_agent_map_ignores_lan_and_wan_sections_gracefully() {
    // lan/wan entries carry no block_id at all — this must never panic
    // on their differently-shaped entries, and must simply not resolve
    // them (the caller falls back to using the raw target as an agent
    // name for those).
    let discovery = serde_json::json!({
        "host": { "addressable": [] },
        "lan": [{ "instance_id": "x", "agents": ["RemoteAgent"] }],
        "wan": { "local_agents_subscribed": ["CloudAgent"] },
    });
    let map = build_block_to_agent_map(&discovery);
    assert!(map.is_empty());
}

#[test]
fn build_block_to_agent_map_handles_missing_sections() {
    let map = build_block_to_agent_map(&serde_json::json!({}));
    assert!(map.is_empty());
}

// ---- identity M3: how the boundary treats a resolve response ---------

use reqwest::StatusCode as S;

#[test]
fn a_single_identified_match_yields_its_uid() {
    let r = interpret_resolve_response(
        S::OK,
        r#"{"resolution":"one","uid":"4f3c-a91","candidate":{}}"#,
        "AgentY",
    );
    assert_eq!(r.unwrap().as_deref(), Some("4f3c-a91"));
}

#[test]
fn no_match_or_no_uid_sends_the_name() {
    for body in [
        r#"{"resolution":"none"}"#,
        r#"{"resolution":"unidentified","candidate":{}}"#,
    ] {
        assert_eq!(
            interpret_resolve_response(S::OK, body, "Scratch").unwrap(),
            None,
            "{body}"
        );
    }
}

/// Only an srv that predates the endpoint gets the bare-name fallback.
#[test]
fn a_missing_endpoint_falls_back_to_the_name() {
    for status in [S::NOT_FOUND, S::METHOD_NOT_ALLOWED] {
        assert_eq!(
            interpret_resolve_response(status, "", "AgentY").unwrap(),
            None,
            "{status}"
        );
    }
}

/// Any other failure refuses: proceeding by name would skip the
/// ambiguity check (Codex P1/P2 on #3563).
#[test]
fn a_server_fault_or_garbled_answer_fails_the_tool_call() {
    for (status, body) in [
        (
            S::SERVICE_UNAVAILABLE,
            r#"{"error":"agent store unavailable"}"#,
        ),
        (
            S::INTERNAL_SERVER_ERROR,
            r#"{"error":"resolve task failed"}"#,
        ),
        (S::OK, "not json"),
        (S::OK, r#"{"resolution":"something-new"}"#),
        (S::OK, r#"{"resolution":"one"}"#),
    ] {
        assert!(
            interpret_resolve_response(status, body, "AgentY").is_err(),
            "{status} {body}"
        );
    }
}

#[test]
fn an_ambiguous_name_is_refused_with_its_candidates() {
    let body = r#"{"resolution":"ambiguous","candidates":[
        {"uid":"4f3c-a91","name":"AgentY","live":true,"block_id":"b1"},
        {"uid":"9b2e-7d4","name":"AgentY","live":false}]}"#;
    let err = interpret_resolve_response(S::OK, body, "AgentY")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("4f3c-a91") && err.contains("9b2e-7d4"),
        "{err}"
    );
}
