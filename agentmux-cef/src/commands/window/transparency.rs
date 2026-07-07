// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window transparency / per-window opacity handlers for the CEF host.
//
// Fourth carve of the commands/window.rs modularization (Plan 1). Pure
// move — no behavior change.
//
// `set_window_transparency`, `set_window_opacity`, `get_window_opacity`
// are `pub` and dispatched by ipc.rs (re-exported `pub use transparency::*`).
// `apply_window_opacity` / `remove_window_opacity` are private Win32 helpers
// used only by the two setters here. `find_all_own_windows` comes from the
// sibling `lifecycle` module (the fallback when a label's HWND hasn't been
// captured yet).
//
// Platform mechanisms (uniform whole-window alpha, post-render, stock-CEF-safe):
//   Windows — WS_EX_LAYERED + SetLayeredWindowAttributes(LWA_ALPHA), inline
//             (Win32 window-style ops are safe from any thread).
//   macOS   — [NSWindow setAlphaValue:], via ui_tasks::post_set_window_alpha
//             (AppKit → must run on the UI thread).
//   Linux   — EWMH _NET_WM_WINDOW_OPACITY (X11/XWayland — the default ozone),
//             via ui_tasks::post_set_window_alpha (CEF Views handle → must run
//             on the UI thread). Native-Wayland ozone: no protocol, no-op.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::state::AppState;

#[cfg(target_os = "windows")]
use super::lifecycle::find_all_own_windows;

/// SPEC_PILLAR1_STEP2 Slice A Phase 2 (reagent P1 on #1985) — trailing-edge
/// debounce for the srv opacity write-through, keyed by window label.
///
/// The frontend's opacity slider calls `set_window_opacity` on every
/// `onInput` tick with no debounce of its own — a drag can fire dozens of
/// calls/sec. The naive approach (spawn a thread + TCP connection per call)
/// lets those writes race each other: srv's `SetWindowOpacity` RPC does an
/// unconditional `store.update` with no sequencing, so out-of-order network
/// delivery could persist a stale mid-drag value instead of the final one
/// — silently defeating the crash-restart correctness this phase exists
/// for. A generation counter per label fixes this without touching the
/// frontend or adding a CAS/version check to the Phase 1 RPC (out of
/// scope for this phase): each call bumps its label's generation, then
/// its spawned writer sleeps `OPACITY_WRITE_DEBOUNCE_MS` and only actually
/// writes if no newer call has superseded it in the meantime. A burst
/// collapses to exactly one outbound write — the final settled value —
/// while deliberate, slowly-spaced changes (deliberate clicks, not a drag)
/// each still get their own write once they clear the window.
const OPACITY_WRITE_DEBOUNCE_MS: u64 = 400;

fn opacity_write_generations() -> &'static Mutex<HashMap<String, u64>> {
    static MAP: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bump and return the new generation for `label`.
fn next_opacity_write_generation(label: &str) -> u64 {
    let mut m = opacity_write_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let gen = m.entry(label.to_string()).or_insert(0);
    *gen += 1;
    *gen
}

/// True if `generation` is still the latest recorded generation for
/// `label` — i.e. no newer `set_window_opacity` call for this label has
/// happened since this one was scheduled.
fn is_current_opacity_write_generation(label: &str, generation: u64) -> bool {
    let m = opacity_write_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    m.get(label).copied() == Some(generation)
}

/// Set window transparency/blur effects for a single window.
///
/// Targets exactly the window identified by `label` (from the frontend's URL
/// `windowLabel` param). Uses the `window_hwnds` map populated by
/// `capture_hwnd_for_label`. Falls back to `find_all_own_windows()` only
/// if the label's HWND hasn't been captured yet (e.g. very early startup).
pub fn set_window_transparency(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let transparent = args.get("transparent").and_then(|v| v.as_bool()).unwrap_or(false);
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.8);
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main").to_string();
    tracing::info!("set_window_transparency: label={} transparent={} opacity={}", label, transparent, opacity);

    #[cfg(target_os = "windows")]
    {
        let hwnd_raw = state.window_hwnds.lock().get(label.as_str()).copied();
        let hwnds: Vec<*mut std::ffi::c_void> = if let Some(raw) = hwnd_raw {
            vec![raw as *mut std::ffi::c_void]
        } else {
            // HWND not yet captured (early startup). Fall back to process
            // enumeration as a best-effort. This path is temporary — once
            // set_window_init_status fires, future calls use the map.
            tracing::warn!("set_window_transparency: no hwnd for label={}, falling back to find_all_own_windows", label);
            find_all_own_windows()
        };
        for hwnd in hwnds {
            unsafe {
                if transparent {
                    apply_window_opacity(hwnd, opacity);
                } else {
                    remove_window_opacity(hwnd);
                }
            }
            tracing::info!("set_window_transparency: applied to {:?}", hwnd);
        }
    }

    // macOS — Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01: the
    // WindowServer analogue of the Win32 layered-window alpha above.
    // [NSWindow setAlphaValue:] fades the entire finished window (content,
    // chrome, and shadow) over the desktop — uniform whole-window alpha,
    // exactly the effect Windows ships. Needs no CEF/renderer cooperation.
    // Per-pixel ("glass") transparency is the separate patched-libcef track.
    #[cfg(target_os = "macos")]
    {
        let alpha = if transparent { opacity.clamp(0.0, 1.0) } else { 1.0 };
        crate::ui_tasks::post_set_window_alpha(state, &label, alpha);
    }

    // Linux — Track 1 (SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01): EWMH
    // _NET_WM_WINDOW_OPACITY, the X11/XWayland analogue of the Win32 layered
    // window and NSWindow.alphaValue arms above. Native-Wayland ozone has no
    // uniform-alpha protocol; the UI task warns and no-ops there.
    #[cfg(target_os = "linux")]
    {
        let alpha = if transparent { opacity.clamp(0.0, 1.0) } else { 1.0 };
        crate::ui_tasks::post_set_window_alpha(state, &label, alpha);
    }

    let _ = state;

    Ok(serde_json::Value::Null)
}


/// Apply window-level opacity via WS_EX_LAYERED + SetLayeredWindowAttributes.
/// This makes the entire window semi-transparent (content + chrome).
#[cfg(target_os = "windows")]
unsafe fn apply_window_opacity(hwnd: *mut std::ffi::c_void, opacity: f64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;

    // Add WS_EX_LAYERED extended style
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as isize);

    // LWA_ALPHA = 0x02
    let result = SetLayeredWindowAttributes(hwnd, 0, alpha, 0x02);
    if result != 0 {
        tracing::info!("Applied window opacity: {} (alpha={})", opacity, alpha);
    } else {
        tracing::warn!("SetLayeredWindowAttributes failed");
    }
}

/// Remove window opacity — restore to fully opaque by removing WS_EX_LAYERED.
#[cfg(target_os = "windows")]
unsafe fn remove_window_opacity(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    if (ex_style & WS_EX_LAYERED as isize) != 0 {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED as isize));
        tracing::info!("Removed window opacity (WS_EX_LAYERED cleared)");
    }
}

/// Set opacity on exactly one window by label.
///
/// Routes through the host reducer (`HostCommand::SetWindowOpacity`) so the
/// change is auditable. The reducer emits `WindowOpacityApplied`; `host_dispatch`
/// reads the event and calls the Win32 helper directly (Win32 window-style ops
/// are safe from any thread). Replaces the global `set_window_transparency` path
/// for per-window calls.
pub fn set_window_opacity(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("set_window_opacity: missing label")?
        .to_string();
    let opacity = args
        .get("opacity")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    let out = state.host_dispatch(crate::reducer::HostCommand::SetWindowOpacity {
        label: label.clone(),
        opacity: opacity as f32,
    });

    // Apply Win32 side-effect synchronously based on emitted event.
    // Reagent P1 on #868: the reducer emits `WindowOpacityApplied` for
    // opacity < 1.0 (clamped translucent value) and `WindowOpacityCleared`
    // for opacity >= 1.0. Matching only `WindowOpacityApplied` left
    // windows semi-transparent after the user restored full opacity.
    // Match both arms.
    #[cfg(target_os = "windows")]
    for ev in &out.events {
        match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                let hwnd_raw = state.window_hwnds.lock().get(ev_label.as_str()).copied();
                if let Some(raw) = hwnd_raw {
                    let hwnd = raw as *mut std::ffi::c_void;
                    unsafe { apply_window_opacity(hwnd, *ev_opacity as f64); }
                } else {
                    tracing::warn!("[opacity] set_window_opacity: no hwnd for label={}", ev_label);
                }
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                let hwnd_raw = state.window_hwnds.lock().get(ev_label.as_str()).copied();
                if let Some(raw) = hwnd_raw {
                    let hwnd = raw as *mut std::ffi::c_void;
                    unsafe { remove_window_opacity(hwnd); }
                } else {
                    tracing::warn!("[opacity] set_window_opacity: no hwnd for label={} (clear)", ev_label);
                }
            }
            _ => {}
        }
    }

    // macOS mirror of the Windows arms above — same reducer events, same
    // both-arms requirement (reagent P1 on #868: matching only Applied left
    // windows semi-transparent after restore). Applies NSWindow.alphaValue
    // on the UI thread via SetWindowAlphaTask.
    #[cfg(target_os = "macos")]
    for ev in &out.events {
        match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, *ev_opacity as f64);
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, 1.0);
            }
            _ => {}
        }
    }

    // Linux mirror — same reducer events, same both-arms requirement
    // (reagent P1 on #868). Applies _NET_WM_WINDOW_OPACITY on the UI thread
    // via SetWindowAlphaTask.
    #[cfg(target_os = "linux")]
    for ev in &out.events {
        match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, *ev_opacity as f64);
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, 1.0);
            }
            _ => {}
        }
    }

    // SPEC_PILLAR1_STEP2 Slice A Phase 2 — write-through to srv's durable
    // `Window.opacity` mirror (added in Phase 1, #1982) so a crashed/
    // restarted host can restore it instead of defaulting to fully opaque
    // (`get_window_opacity` below reads it back). Platform-agnostic:
    // `out.events` is the same regardless of which cfg-gated block above
    // ran, so this fires once per call, not once per platform.
    //
    // Debounced per label (see `next_opacity_write_generation` above,
    // reagent P1 on #1985) so a rapid burst — e.g. a slider drag, which
    // the frontend fires on every `onInput` tick with no debounce of its
    // own — collapses to exactly one outbound write of the final settled
    // value, instead of one thread + TCP connection per tick racing to
    // persist whichever happens to land last over the network. Silently
    // skipped when the label has no registered `backend_window_id` (e.g.
    // a pre-promote pool window, or a floating pane — see
    // SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md §1.B,
    // floating panes have no srv `Window` row to write to at all).
    for ev in &out.events {
        let resolved = match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                Some((ev_label.clone(), Some(*ev_opacity)))
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                Some((ev_label.clone(), None))
            }
            _ => None,
        };
        let Some((ev_label, ev_opacity)) = resolved else { continue };
        let Some(window_id) = state.backend_window_id(&ev_label) else {
            tracing::debug!(
                label = %ev_label,
                "[opacity] set_window_opacity: no backend_window_id — skipping srv write-through"
            );
            continue;
        };
        let generation = next_opacity_write_generation(&ev_label);
        let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
        let auth_key = state.auth_key.lock().clone();
        let debounce_label = ev_label.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(OPACITY_WRITE_DEBOUNCE_MS));
            if !is_current_opacity_write_generation(&debounce_label, generation) {
                // A newer call for this label superseded us during the
                // sleep — this value is stale, don't persist it.
                return;
            }
            crate::client::backend_set_window_opacity(&web_endpoint, &auth_key, &window_id, ev_opacity);
        });
    }

    let _ = out;
    Ok(serde_json::Value::Null)
}

/// Return the currently tracked opacity for a label.
///
/// Reads from `HostState.window_opacities` — reflects the last value applied
/// via `set_window_opacity`, not the Win32 layer. Used by the frontend to
/// restore opacity on window init without an extra IPC round-trip.
///
/// SPEC_PILLAR1_STEP2 Slice A Phase 2 — a miss here means either "never set"
/// OR "fresh process after a crash/restart" (`window_opacities` starts
/// empty every launch). Falls back to srv's durable `Window.opacity` mirror
/// via `backend_get_window_opacity` in the latter case — the actual
/// reproject payoff of Phase 1's write-through. `async` (unlike the sibling
/// handlers in this file) so the blocking network read runs on tokio's
/// blocking-thread pool (`spawn_blocking`) rather than the async worker
/// thread handling this IPC request.
pub async fn get_window_opacity(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    let host_memory = state.host_state.lock().window_opacities.get(&label).copied();
    if let Some(opacity) = host_memory {
        return Ok(serde_json::json!(opacity));
    }

    let opacity = match state.backend_window_id(&label) {
        Some(window_id) => {
            let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
            let auth_key = state.auth_key.lock().clone();
            tokio::task::spawn_blocking(move || {
                crate::client::backend_get_window_opacity(&web_endpoint, &auth_key, &window_id)
            })
            .await
            .ok()
            .flatten()
            .unwrap_or(1.0)
        }
        None => 1.0,
    };
    Ok(serde_json::json!(opacity))
}

#[cfg(test)]
mod opacity_debounce_tests {
    use super::{is_current_opacity_write_generation, next_opacity_write_generation};

    /// Simulates a rapid burst (slider drag): only the LAST call's
    /// generation should still be "current" once all bumps have happened
    /// — the reagent P1 scenario this debounce exists to prevent (an
    /// earlier, superseded call's stale value winning a network race).
    #[test]
    fn burst_only_the_last_generation_is_current() {
        let label = "test-burst-only-last-wins";
        let gens: Vec<u64> = (0..5).map(|_| next_opacity_write_generation(label)).collect();
        for &g in &gens[..gens.len() - 1] {
            assert!(
                !is_current_opacity_write_generation(label, g),
                "earlier generation {g} must be superseded after a burst"
            );
        }
        assert!(
            is_current_opacity_write_generation(label, *gens.last().unwrap()),
            "the last generation in the burst must still be current"
        );
    }

    /// A single, deliberate (non-burst) call must still be current —
    /// debouncing a burst down to one write must not also drop the only
    /// write when there's no burst to coalesce.
    #[test]
    fn single_call_is_current() {
        let label = "test-single-call-is-current";
        let g = next_opacity_write_generation(label);
        assert!(is_current_opacity_write_generation(label, g));
    }

    /// Generations are tracked independently per label — a burst on one
    /// window must not supersede a pending write for a different window.
    #[test]
    fn generations_are_independent_per_label() {
        let a = "test-independent-label-a";
        let b = "test-independent-label-b";
        let ga = next_opacity_write_generation(a);
        let gb = next_opacity_write_generation(b);
        // A second call on `a` only supersedes `a`'s generation, not `b`'s.
        next_opacity_write_generation(a);
        assert!(!is_current_opacity_write_generation(a, ga));
        assert!(is_current_opacity_write_generation(b, gb));
    }

    /// A generation that was never issued for a label (or an unknown
    /// label entirely) must never read as current — the debounce map
    /// only knows about labels it has actually seen.
    #[test]
    fn unknown_label_or_generation_is_never_current() {
        assert!(!is_current_opacity_write_generation("test-never-seen-this-label", 1));
        let label = "test-known-label-wrong-generation";
        let g = next_opacity_write_generation(label);
        assert!(!is_current_opacity_write_generation(label, g + 1));
    }
}
