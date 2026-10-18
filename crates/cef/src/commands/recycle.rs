// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Recycling a window's renderer: reloading its page to give back the memory
//! Blink's heap keeps after a long session
//! (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §3).
//!
//! The page drives the recycle. It checks what would be lost, saves its state,
//! then calls `recycle_begin`: the host measures the window's renderer, keeps a
//! picture of the window to cover the reload, and logs the start. The page
//! reloads; the new document calls `take_reload_snapshot` while it boots, which
//! hands back the picture and, 60 s later, logs the renderer's settled size.
//!
//! The IPC router has no browser context, so the page passes its own label. Any
//! app page could therefore name another window and take its pending cover;
//! only the app's own pages hold the IPC token, so that is accepted.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::state::AppState;

/// A recycle whose new document never asked for its picture (the reload
/// failed, or the window closed) is dropped after this long.
const BACKSTOP: Duration = Duration::from_secs(60);

/// How long after the new document starts that its size counts as settled.
const SETTLE: Duration = Duration::from_secs(60);

/// JPEG quality of the cover picture.
const COVER_QUALITY: u8 = 70;

struct Pending {
    at: Instant,
    reason: String,
    before_mb: Option<u64>,
    cover: Option<String>,
}

static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn put(label: &str, pending: Pending) {
    let mut map = PENDING.lock().unwrap();
    map.retain(|_, p| p.at.elapsed() <= BACKSTOP);
    map.insert(label.to_string(), pending);
}

fn take(label: &str) -> Option<Pending> {
    let mut map = PENDING.lock().unwrap();
    map.retain(|_, p| p.at.elapsed() <= BACKSTOP);
    map.remove(label)
}

/// The renderer serving `label` alone, from the renderer map. `None` when the
/// window has no reported renderer, or shares it with another window: a reload
/// then frees only part of what the process holds, so its size says nothing
/// about this window (spec §4.1).
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn own_renderer_pid(entries: &[crate::renderer_map::RendererEntry], label: &str) -> Option<u32> {
    let pid = entries.iter().find(|e| e.label == label)?.pid;
    let shared = entries.iter().any(|e| e.pid == pid && e.label != label);
    (!shared).then_some(pid)
}

/// Private memory of `label`'s renderer in MB (private working set on
/// Windows, Tower's metric). Not on Linux: the renderer map holds the
/// sandbox-namespace PID there, which only srv translates (Tower), so a match
/// against host PIDs could name another process.
#[cfg(not(target_os = "linux"))]
pub(crate) fn renderer_mb(state: &AppState, label: &str) -> Option<u64> {
    let pid = own_renderer_pid(&crate::renderer_map::snapshot(state), label)?;
    let procs = agentmux_procstats::snapshot().ok()?;
    let bytes = procs.iter().find(|p| p.pid == pid)?.mem_private?;
    Some(bytes / (1024 * 1024))
}

#[cfg(target_os = "linux")]
pub(crate) fn renderer_mb(_state: &AppState, _label: &str) -> Option<u64> {
    None
}

/// Whether the window is on screen, so a picture of it can cover the reload.
#[cfg(target_os = "windows")]
fn on_screen(state: &AppState, label: &str) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    let Some(hwnd) = state.window_hwnd(label) else { return false };
    // SAFETY: both tolerate stale or invalid handles (they return FALSE).
    unsafe { IsWindowVisible(hwnd as HWND) != 0 && IsIconic(hwnd as HWND) == 0 }
}

#[cfg(not(target_os = "windows"))]
fn on_screen(_state: &AppState, _label: &str) -> bool {
    true
}

/// `recycle_begin { label, reason }`: the page has saved its state and is about
/// to reload. Measure, keep the cover picture, log.
pub async fn recycle_begin(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("recycle_begin: label is required")?;
    if !state.is_live_top_level_browser(label) {
        return Err(format!("recycle_begin: {label} is not an app window"));
    }
    let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("command").to_string();
    let before_mb = renderer_mb(state, label);
    let cover = if on_screen(state, label) {
        crate::browser_api::routes::capture_window_viewport(state, label, COVER_QUALITY)
            .await
            .map_err(|e| tracing::warn!(target: "recycle", label, error = %e, "[recycle] no cover picture"))
            .ok()
    } else {
        None
    };
    tracing::info!(
        target: "recycle",
        label,
        reason = %reason,
        before_mb,
        cover = cover.is_some(),
        "[recycle] {label} {reason} {} MB",
        before_mb.map_or("?".to_string(), |mb| mb.to_string()),
    );
    put(label, Pending { at: Instant::now(), reason, before_mb, cover });
    Ok(serde_json::json!({ "before_mb": before_mb }))
}

/// `recycle_cancel { label }`: something the reload would lose started while
/// the page prepared, so it won't reload. Drop the pending recycle, so a later
/// ordinary load of the window doesn't show its picture.
pub fn recycle_cancel(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("recycle_cancel: label is required")?;
    if take(label).is_some() {
        tracing::info!(target: "recycle", label, "[recycle] {label} cancelled before the reload");
    }
    Ok(serde_json::Value::Null)
}

/// `take_reload_snapshot { label }`: the new document is booting. Hand back
/// the cover picture, if this load is a recycle, and log the settled size later.
pub fn take_reload_snapshot(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("take_reload_snapshot: label is required")?;
    // Every load of the window comes here: its governor baseline starts over.
    super::recycle_governor::note_window_loaded(label);
    let Some(pending) = take(label) else {
        return Ok(serde_json::json!({ "jpeg_base64": null }));
    };
    let state = Arc::clone(state);
    let owned = label.to_string();
    std::thread::Builder::new()
        .name("recycle-settle".into())
        .spawn(move || {
            std::thread::sleep(SETTLE);
            let after_mb = renderer_mb(&state, &owned);
            tracing::info!(
                target: "recycle",
                label = %owned,
                reason = %pending.reason,
                before_mb = pending.before_mb,
                after_mb,
                "[recycle] {owned} settled at {} MB",
                after_mb.map_or("?".to_string(), |mb| mb.to_string()),
            );
        })
        .map_err(|e| format!("take_reload_snapshot: {e}"))?;
    Ok(serde_json::json!({ "jpeg_base64": pending.cover }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer_map::RendererEntry;

    fn entry(pid: u32, label: &str) -> RendererEntry {
        RendererEntry { pid, label: label.to_string(), block_id: None, pool: false, window_id: None }
    }

    #[test]
    fn a_window_with_its_own_renderer_is_measured() {
        let entries = [entry(10, "main"), entry(20, "window-1")];
        assert_eq!(own_renderer_pid(&entries, "main"), Some(10));
        assert_eq!(own_renderer_pid(&entries, "window-1"), Some(20));
    }

    #[test]
    fn a_shared_or_unknown_renderer_is_not() {
        let entries = [entry(10, "main"), entry(10, "window-1"), entry(30, "floating-2")];
        assert_eq!(own_renderer_pid(&entries, "main"), None, "shared with window-1");
        assert_eq!(own_renderer_pid(&entries, "window-9"), None, "no renderer reported");
    }

    #[test]
    fn the_new_document_takes_its_cover_once() {
        let label = "recycle-test-take-once";
        put(label, Pending { at: Instant::now(), reason: "command".into(), before_mb: Some(900), cover: Some("jpeg".into()) });
        let first = take(label).expect("the pending recycle");
        assert_eq!(first.cover.as_deref(), Some("jpeg"));
        assert_eq!(first.before_mb, Some(900));
        assert!(take(label).is_none(), "a later ordinary load of the window is not a recycle");
    }

    #[test]
    fn a_cancelled_recycle_leaves_nothing_for_the_next_load() {
        let label = "recycle-test-cancel";
        put(label, Pending { at: Instant::now(), reason: "command".into(), before_mb: None, cover: Some("jpeg".into()) });
        recycle_cancel(&serde_json::json!({ "label": label })).unwrap();
        assert!(take(label).is_none());
    }

    #[test]
    fn a_recycle_that_never_reloaded_is_dropped() {
        let label = "recycle-test-backstop";
        let stale = Instant::now().checked_sub(BACKSTOP + Duration::from_secs(1)).expect("clock far enough from boot");
        put(label, Pending { at: stale, reason: "command".into(), before_mb: None, cover: None });
        assert!(take(label).is_none());
    }
}
