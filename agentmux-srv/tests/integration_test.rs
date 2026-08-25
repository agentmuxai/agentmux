// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// RAII guard around the test-spawned srv (SPEC_TEST_SRV_SPAWN_GUARDS_2026_07_11).
///
/// Kills and reaps the child on drop — including drop-during-panic-unwind,
/// which is exactly when an explicit end-of-test `kill()` never runs: a
/// failed assertion used to leave the srv process (its two listening
/// sockets, its SQLite store, its shell children) alive indefinitely on the
/// dev machine / CI runner.
///
/// On Windows the child is ALSO assigned to an anonymous kill-on-close Job
/// Object at spawn time, for two failure modes `Drop` cannot cover:
/// - **Grandchildren**: srv spawns its own children (live-observed
///   2026-07-11: a leaked `--crash-monitor` grandchild held a shell
///   pipeline's inherited stdout handle open for 20+ minutes after `cargo
///   test` exited). Killing the direct child does not reap them; closing
///   the job handle kills the whole tree.
/// - **Hard kill of the test process itself** (CI timeout, Ctrl+C): the OS
///   closes the job handle with the process, cascading to the tree.
///
/// The job is anonymous, created by this test, and can only ever contain
/// processes this test spawned — upholding isolation invariants I2/I3
/// (never touch a job/process this code did not create).
struct SrvGuard {
    child: std::process::Child,
    #[cfg(windows)]
    _job: Option<JobHandle>,
}

impl Drop for SrvGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait(); // reap — no zombie on unix
        // Windows: `_job` drops after this, closing the handle → the OS
        // kills anything left in the tree (grandchildren included).
    }
}

impl std::ops::Deref for SrvGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &std::process::Child {
        &self.child
    }
}

impl std::ops::DerefMut for SrvGuard {
    fn deref_mut(&mut self) -> &mut std::process::Child {
        &mut self.child
    }
}

/// Owned kill-on-close Job Object handle. Same call shapes as the
/// launcher's production J0 (`agentmux-launcher/src/job_object.rs`), kept
/// test-local rather than exported across crates — the needed surface is
/// this small.
#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

// SAFETY: a job HANDLE is an opaque kernel handle with no documented thread
// affinity (CloseHandle may be called from any thread) — same justification
// as the launcher's JobHandle.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
fn assign_to_kill_on_close_job(child: &std::process::Child) -> Option<JobHandle> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            eprintln!("[srv-guard] CreateJobObjectW failed — Drop-only cleanup for this child");
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            eprintln!("[srv-guard] SetInformationJobObject failed — Drop-only cleanup");
            return None;
        }
        if AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
            CloseHandle(job);
            eprintln!("[srv-guard] AssignProcessToJobObject failed — Drop-only cleanup");
            return None;
        }
        Some(JobHandle(job))
    }
}

/// Helper: spawn agentmux-srv as a subprocess and parse AGENTMUXSRV-ESTART.
/// Returns (guard, web_addr, ws_addr, auth_key). The guard owns cleanup —
/// tests must NOT call `kill()` for cleanup (scope exit / panic unwind does
/// it, and on Windows the job object reaps the whole tree).
fn spawn_backend() -> (SrvGuard, String, String, String) {
    let auth_key = "integration-test-key-12345";

    let binary = env!("CARGO_BIN_EXE_agentmux-srv");

    let child = Command::new(binary)
        .env("AGENTMUX_AUTH_KEY", auth_key)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn agentmux-srv");

    #[cfg(windows)]
    let job = assign_to_kill_on_close_job(&child);

    // Guard is constructed IMMEDIATELY after spawn — the ESTART parsing
    // below can panic (assert/expect), and a panic before the guard exists
    // would leak the child, which is the exact defect this guard fixes.
    let mut guard = SrvGuard {
        child,
        #[cfg(windows)]
        _job: job,
    };

    let stderr = guard.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    let mut web_addr = String::new();
    let mut ws_addr = String::new();

    for line in reader.lines() {
        let line = line.expect("failed to read stderr");
        if line.contains("AGENTMUXSRV-ESTART") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in &parts {
                if let Some(addr) = part.strip_prefix("ws:") {
                    ws_addr = addr.to_string();
                } else if let Some(addr) = part.strip_prefix("web:") {
                    web_addr = addr.to_string();
                }
            }
            break;
        }
    }

    assert!(!web_addr.is_empty(), "failed to parse web addr from ESTART");
    assert!(!ws_addr.is_empty(), "failed to parse ws addr from ESTART");

    (guard, web_addr, ws_addr, auth_key.to_string())
}

#[test]
fn health_returns_200() {
    // No manual cleanup in these tests: SrvGuard kills + reaps on scope
    // exit — INCLUDING panic unwind on a failed assert, which the old
    // explicit end-of-test kill() never covered.
    let (_child, web_addr, _ws_addr, _auth_key) = spawn_backend();

    let url = format!("http://{}/", web_addr);
    let resp = reqwest::blocking::get(&url).expect("health request failed");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().unwrap();
    assert_eq!(body["status"], "ok");
}

#[test]
fn auth_rejects_missing_key() {
    let (_child, web_addr, _ws_addr, _auth_key) = spawn_backend();

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("http://{}/agentmux/service", web_addr))
        .send()
        .expect("request failed");
    assert_eq!(resp.status(), 401);
}

#[test]
fn auth_accepts_valid_header() {
    let (_child, web_addr, _ws_addr, auth_key) = spawn_backend();

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("http://{}/agentmux/service", web_addr))
        .header("X-AuthKey", &auth_key)
        .header("Content-Type", "application/json")
        .body(r#"{"service":"client","method":"GetClientData"}"#)
        .send()
        .expect("request failed");
    assert_eq!(resp.status(), 200); // real handler returns 200

    let body: serde_json::Value = resp.json().unwrap();
    assert!(body["success"].as_bool().unwrap_or(false));
}

/// Same as `spawn_backend`, but pins `AGENTMUX_DATA_DIR` to a caller-owned
/// directory instead of whatever this process would otherwise default to —
/// so a second spawn against the same path can prove state survived a real
/// process exit, not just an in-process code path.
fn spawn_backend_with_data_dir(data_dir: &std::path::Path) -> (SrvGuard, String, String, String) {
    let auth_key = "integration-test-key-12345";
    let binary = env!("CARGO_BIN_EXE_agentmux-srv");

    let child = Command::new(binary)
        .env("AGENTMUX_AUTH_KEY", auth_key)
        .env("AGENTMUX_DATA_DIR", data_dir)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn agentmux-srv");

    #[cfg(windows)]
    let job = assign_to_kill_on_close_job(&child);

    let mut guard = SrvGuard {
        child,
        #[cfg(windows)]
        _job: job,
    };

    let stderr = guard.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    let mut web_addr = String::new();
    let mut ws_addr = String::new();

    for line in reader.lines() {
        let line = line.expect("failed to read stderr");
        if line.contains("AGENTMUXSRV-ESTART") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in &parts {
                if let Some(addr) = part.strip_prefix("ws:") {
                    ws_addr = addr.to_string();
                } else if let Some(addr) = part.strip_prefix("web:") {
                    web_addr = addr.to_string();
                }
            }
            break;
        }
    }

    assert!(!web_addr.is_empty(), "failed to parse web addr from ESTART");
    assert!(!ws_addr.is_empty(), "failed to parse ws addr from ESTART");

    (guard, web_addr, ws_addr, auth_key.to_string())
}

/// Minimal RPC helper: POST to `/agentmux/service`, assert success, return `data`.
fn rpc(
    client: &reqwest::blocking::Client,
    web_addr: &str,
    auth_key: &str,
    service: &str,
    method: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({ "service": service, "method": method, "args": args });
    let resp = client
        .post(format!("http://{}/agentmux/service", web_addr))
        .header("X-AuthKey", auth_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .unwrap_or_else(|e| panic!("{}::{} request failed: {}", service, method, e));
    let status = resp.status();
    let json: serde_json::Value = resp.json().expect("response body was not JSON");
    assert!(
        status.is_success() && json["success"].as_bool().unwrap_or(false),
        "{}::{} failed: status={} body={}",
        service,
        method,
        status,
        json
    );
    json["data"].clone()
}

/// End-to-end proof of SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
/// Feature 1 — spawns two SEPARATE real `agentmux-srv` processes against the
/// same data dir (not two calls within one process), matching what an actual
/// quit-and-relaunch does: process 1 creates a window, adds a block with a
/// distinctive meta marker, and is closed gracefully (`CloseWindow`, which
/// snapshots-then-cascades — see `window_close::handle_close_window`); it is
/// then killed for real. Process 2 starts cold against the same
/// `AGENTMUX_DATA_DIR` and calls `CreateWindow` the same way the frontend's
/// genuine cold-start path does (`restoreIfAvailable: true`, `app-init.ts`).
/// The marker surviving into process 2's restored block proves the durable
/// write-through and the replay path both work against the real HTTP API and
/// real SQLite file, not just the in-process unit tests in
/// `server::service::session_restore`.
#[test]
fn restore_last_session_survives_a_real_process_restart() {
    let data_dir = tempfile::tempdir().expect("failed to create temp data dir");
    let client = reqwest::blocking::Client::new();
    let marker = "hello-restore-e2e";

    // --- Process 1: create a window, add a marked block, close gracefully ---
    let (guard1, web_addr, _ws_addr, auth_key) = spawn_backend_with_data_dir(data_dir.path());

    // Mirror the real frontend cold-start path (`initHostWave` in
    // `app-init.ts`): a fresh server has already bootstrapped a "Starter
    // workspace" window via `ensure_initial_data` at startup, so
    // `Client.windowids` is non-empty here — the frontend reuses
    // `windowids[0]` rather than calling `CreateWindow` again. Calling
    // `CreateWindow` unconditionally here (as this test used to) would spawn
    // a SECOND window that the bootstrap window still keeps alive,
    // permanently pinning `Client.windowids` above one entry — CloseWindow
    // would never see it empty out, so it (correctly, per
    // `will_empty_windowids`) would never write a snapshot.
    let client_data = rpc(&client, &web_addr, &auth_key, "client", "GetClientData", serde_json::json!([]));
    let window_id = client_data["windowids"][0]
        .as_str()
        .expect("bootstrap window id")
        .to_string();
    let window = rpc(&client, &web_addr, &auth_key, "window", "GetWindow", serde_json::json!([window_id]));
    let workspace_id = window["workspaceid"].as_str().expect("window workspaceid").to_string();

    let workspace = rpc(
        &client,
        &web_addr,
        &auth_key,
        "object",
        "GetObject",
        serde_json::json!([format!("workspace:{}", workspace_id)]),
    );
    // The bootstrap workspace's initial tab can still be sitting in the
    // legacy `pinnedtabids` field rather than `tabids` (drained into
    // `tabids` only on the next `TabsReordered` event) — check both, same
    // as `session_restore::snapshot_workspace`.
    let tab_id = workspace["tabids"][0]
        .as_str()
        .or_else(|| workspace["pinnedtabids"][0].as_str())
        .expect("workspace has a tab")
        .to_string();

    let marker_block = rpc(
        &client,
        &web_addr,
        &auth_key,
        "object",
        "CreateBlock",
        serde_json::json!([{ "meta": { "view": "term", "test:marker": marker } }, null, tab_id]),
    );
    // `object::CreateBlock` returns the new block id as a bare JSON string
    // (`success_data_updates(json!(bid), updates)`), not a block object.
    let marker_block_id = marker_block.as_str().expect("marker block id string").to_string();
    assert!(!marker_block_id.is_empty());

    rpc(
        &client,
        &web_addr,
        &auth_key,
        "window",
        "CloseWindow",
        serde_json::json!([window_id]),
    );

    // Kill the process for real (not just logically closed) — proves this
    // survives an actual restart, not merely an in-process code path.
    drop(guard1);

    // --- Process 2: fresh process, SAME data dir — cold start must restore ---
    let (guard2, web_addr2, _ws_addr2, auth_key2) = spawn_backend_with_data_dir(data_dir.path());

    let restored_window = rpc(
        &client,
        &web_addr2,
        &auth_key2,
        "window",
        "CreateWindow",
        serde_json::json!([null, "", "main", true]),
    );
    let restored_ws_id = restored_window["workspaceid"]
        .as_str()
        .expect("restored window workspaceid")
        .to_string();
    assert_ne!(
        restored_ws_id, workspace_id,
        "restore creates a brand-new workspace, it does not resurrect the deleted one"
    );

    let restored_workspace = rpc(
        &client,
        &web_addr2,
        &auth_key2,
        "object",
        "GetObject",
        serde_json::json!([format!("workspace:{}", restored_ws_id)]),
    );
    let restored_tab_id = restored_workspace["tabids"][0]
        .as_str()
        .expect("restored workspace has a tab")
        .to_string();
    let restored_tab = rpc(
        &client,
        &web_addr2,
        &auth_key2,
        "object",
        "GetObject",
        serde_json::json!([format!("tab:{}", restored_tab_id)]),
    );
    let restored_block_ids: Vec<String> = restored_tab["blockids"]
        .as_array()
        .expect("restored tab has blockids")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    // The default seed is 4 blocks (agent/swarm/armory/sysinfo,
    // SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md); our marker block was the
    // 5th added before close, so a correct restore must bring all 5 back,
    // not silently drop the one added after the initial seed.
    assert_eq!(restored_block_ids.len(), 5, "expected all 5 original blocks to be restored");

    let mut found_marker = false;
    for block_id in &restored_block_ids {
        let block = rpc(
            &client,
            &web_addr2,
            &auth_key2,
            "object",
            "GetObject",
            serde_json::json!([format!("block:{}", block_id)]),
        );
        if block["meta"]["test:marker"].as_str() == Some(marker) {
            found_marker = true;
            // And it must be a genuinely NEW block id, not the one from the
            // now-deleted first workspace (CreateBlock always mints a fresh
            // id; this also guards against a remap bug leaving a stale
            // placeholder or the original id in the restored tree).
            assert_ne!(block_id, &marker_block_id);
            break;
        }
    }
    assert!(
        found_marker,
        "restored session must contain the marker block created before the \
         process restart; got block ids: {:?}",
        restored_block_ids
    );

    drop(guard2);
}

#[test]
fn sigterm_exits_process() {
    let (mut child, _web_addr, _ws_addr, _auth_key) = spawn_backend();

    // Send SIGTERM (matching Go's graceful shutdown)
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not exit within 5s after SIGTERM");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("try_wait error: {}", e),
        }
    }
}
