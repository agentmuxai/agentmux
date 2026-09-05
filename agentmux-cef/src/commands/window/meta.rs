// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window metadata / misc command handlers for the CEF host — zoom,
// label / is-main queries, double-click time, instance + window listing,
// focus, backend-window registration, and DevTools toggles.
//
// Fifth carve of the commands/window.rs modularization (Plan 1). All
// handlers are `pub` and dispatched by ipc.rs (re-exported
// `pub use meta::*`). Pure move — no behavior change. Self-contained:
// every cross-module reference is a fully-qualified `crate::…` path
// (events / ui_tasks / client / launcher_ipc), so no helper imports.

use std::sync::Arc;

use crate::state::AppState;

/// Get the current zoom factor.
pub fn get_zoom_factor(state: &Arc<AppState>) -> serde_json::Value {
    let factor = *state.zoom_factor.lock();
    serde_json::json!(factor)
}

/// Set the zoom factor.
/// CEF zoom uses a logarithmic scale: zoom_level = log2(zoom_factor)
/// So factor 1.0 = level 0, factor 2.0 = level 1, factor 0.5 = level -1
pub fn set_zoom_factor(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let factor = args
        .get("factor")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing factor".to_string())?;

    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock() = factor;

    // Convert to CEF zoom level (log base 1.2)
    // CEF uses: zoom_factor = 1.2 ^ zoom_level
    // So: zoom_level = log(zoom_factor) / log(1.2)
    let zoom_level = factor.ln() / 1.2_f64.ln();

    // NOTE: host.set_zoom_level() deadlocks from IPC thread, and post_task
    // crashes with current CEF bindings. Zoom is applied via CSS on the frontend.
    // The zoom_factor state is stored for get_zoom_factor queries.

    // Emit zoom-factor-change event
    crate::events::emit_event_from_state(state, "zoom-factor-change", &serde_json::json!(factor));

    Ok(serde_json::Value::Null)
}





/// Get the current window label.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn get_window_label(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label)
}

/// Check if this is the main window.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn is_main_window(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label == "main")
}

/// Return the OS double-click interval in milliseconds.
/// On Windows: `GetDoubleClickTime()` — typically 500ms, user-configurable
/// via Mouse settings. On non-Windows: hardcoded 500ms (the Win32 default,
/// also a common cross-platform default; Phase 7 can refine per platform).
///
/// Used by the InstancePanel to defer single-click focus past the user's
/// dblclick threshold so dblclick-to-rename works for everyone, not just
/// users with the default-or-faster setting. Without this query, a fixed
/// constant would make rename unreliable for slow double-clickers
/// (codex PR #569 round-2 P2).
pub fn get_double_click_time() -> serde_json::Value {
    #[cfg(target_os = "windows")]
    {
        let ms = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        serde_json::json!(ms)
    }
    #[cfg(not(target_os = "windows"))]
    {
        serde_json::json!(500u32)
    }
}

/// List all open window instances with their backend window IDs.
/// Same filtering as `list_windows` (excludes unpromoted pool windows
/// and browser-pane child HWNDs), but returns `[{label, windowId}]`
/// pairs so the frontend can resolve per-window backend objects
/// (Window record → meta["window:displayname"], etc.) without an
/// extra round-trip per row.
///
/// `windowId` is `None` for windows that haven't yet completed the
/// `register_backend_window` round-trip — typically a freshly-spawned
/// window before its frontend has finished init. Callers should
/// fall back to label/index-based naming in that case.
pub fn list_window_instances(state: &Arc<AppState>) -> serde_json::Value {
    // Atomic snapshot — pool inventory + browsers under ONE lock.
    // Two-lock variants race against `promote_pool_window` between
    // the reads and would let a just-promoted user window be
    // excluded (or admit a still-hidden pool window).
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()) && !l.starts_with("browser-pane-"))
        .collect();
    // Read backend window IDs via `state.backend_window_id()`,
    // which queries the launcher-fed `shadow_backend_window_ids`
    // (sole source of truth post-B.5e). Resolve labels OUTSIDE
    // the browsers lock to avoid nesting (browsers + shadow).
    let entries: Vec<serde_json::Value> = labels
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l,
                "windowId": state.backend_window_id(l),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// List all open window labels, excluding unpromoted pool windows.
/// Pool windows are pre-warmed tear-off scratch windows kept hidden
/// from the user (WS_EX_TOOLWINDOW, no taskbar entry). Including them
/// in `list_windows` inflates the frontend's InstancePanel row count
/// with phantom entries the user can't see or focus.
///
/// Uses `state.user_visibility_snapshot()` — atomic read of pool
/// inventory (`unpromoted` ∪ `pool.queue`) and the browser registry
/// under one host_state lock. Both `unpromoted` (populated at spawn
/// time) and `pool.queue` (populated after renderer-ready, before
/// promote) are host-internal: the window is hidden off-screen and
/// has no UI a user could see or focus. The atomic read is required
/// because a two-lock variant races against `promote_pool_window`.
pub fn list_windows(state: &Arc<AppState>) -> serde_json::Value {
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()))
        .collect();
    serde_json::json!(labels)
}

/// Focus a specific window by label.
///
/// Uses the CEF Views `Window::activate()` API on all platforms (via
/// `post_focus_window` → `FocusWindowTask`). On Windows in Views mode,
/// `browser.host().window_handle()` returns NULL — the previous direct
/// SetForegroundWindow path silently failed there. Views' `activate()`
/// resolves the actual top-level HWND through `browser_view_get_for_browser
/// → window()` which is the only correct way to reach it in Views mode.
pub fn focus_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_focus_window(state, label);
    Ok(serde_json::Value::Null)
}

/// Get the instance number for the current window.
///
/// Reads from `state.instance_num()` which queries the launcher-fed
/// `shadow_instance_registry` (B.5e — sole source of truth post-migration).
/// Brief race window for early lookups: see `app-init.ts` retry logic.
pub fn get_instance_number(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    serde_json::json!(state.instance_num(label).unwrap_or(1))
}

/// Register the backend window ID for a window label.
/// Called by the frontend after it has initialized its backend Window object.
/// Used by `on_before_close` to notify the backend when a secondary window closes.
pub fn register_backend_window(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let window_id = args.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
    tracing::info!(label = %label, window_id = %window_id, "[window] register_backend_window received");
    crate::client::dlog(&format!("register_backend_window: label={} window_id={}", label, window_id));
    if !window_id.is_empty() {
        // Phase B.5 (window_id_map step d) — host no longer mutates
        // `window_id_map` locally. The launcher's
        // `state.backend_window_ids` (B.5 step a) is sole authority;
        // we just send the report and the shadow update populates
        // the host-side projection.
        tracing::info!(label = %label, window_id = %window_id, "[window] registered backend window ID");
        crate::launcher_ipc::report_backend_window_id_registered(
            label.to_string(),
            window_id.to_string(),
        );
        // Phase B.7.3.3 — the launcher's
        // `Event::BackendWindowIdRegistered` (delivered via the CEF
        // JS bridge) carries the label → windowId mapping change to
        // every renderer's reducer. No sync emit here.
        //
        // On non-Windows there is NO launcher, so the
        // `Event::BackendWindowIdRegistered` → `apply_event_to_shadow` round
        // trip that fills `shadow_backend_window_ids` never runs
        // (`report_backend_window_id_registered` no-ops without a launcher
        // channel). The host is the sole authority here, so populate the
        // projection directly. `resolve_window_at_cursor` reads it via
        // `backend_window_id(label)` to resolve a redock target's window_id;
        // without this it is always `None` on macOS/Linux and redock silently
        // fails. See docs/analysis/REPORT_MACOS_FLOATING_PANE_REDOCK_2026_05_30.md.
        #[cfg(not(target_os = "windows"))]
        state
            .shadow_backend_window_ids
            .lock()
            .insert(label.to_string(), window_id.to_string());

        // SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P1, PR #2032,
        // 2026-07-08) — this label's own registration is exactly the
        // confirmation `reproject_from_srv` was waiting for: proof this
        // window's frontend actually loaded and round-tripped IPC, not just
        // that a CreateWindowTask was posted. Drain-and-close the OLD srv
        // window_id it was reprojected from, if any. See
        // `AppState::pending_reproject_closures`'s doc comment for why this
        // can't happen any earlier (right after `open_window_with_kind`
        // returns `Ok`) without risking silent data loss.
        let old_reprojected_id = state.pending_reproject_closures.lock().confirm(label);
        if let Some(old_id) = old_reprojected_id {
            let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
            let auth_key = state.auth_key.lock().clone();
            tracing::info!(
                target: "reproject",
                new_label = %label,
                old_window_id = %old_id,
                "[reproject] new window confirmed live — closing the old srv window_id it replaced"
            );
            std::thread::spawn(move || {
                crate::client::backend_close_window(&web_endpoint, &auth_key, &old_id);
            });
        }

        // Workstream 0 Phase 1 prerequisite #2 (issue #2977) — the SAME
        // registration is also the liveness proof a promoted pool window is
        // waiting for. Reuses this signal for exactly the reason the block
        // above does: it proves the renderer loaded, ran JS, and
        // round-tripped IPC. `confirm` is false for every ordinary
        // (non-promoted) registration, which is the common case. See
        // `state::promote_liveness` for why `IsWindow()` alone can't
        // establish this and why `on_load_end` can't be used here.
        if state.promote_liveness.lock().confirm(label) {
            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %label,
                "[pool] promoted window confirmed live (renderer registered its backend window)"
            );
        }

        // SPEC_PILLAR1_STEP3 Phase 2 — write-through this window's kind +
        // parent linkage to srv now that its concrete window_id is known.
        // This is the first point in the window's life where the host has
        // both facts at once: `WindowMeta` (kind/parent_instance_id) was
        // populated at creation time (`on_after_created`, from the
        // pre-create handoff — see the module doc above), and `window_id`
        // just arrived as this call's argument. The frontend never learns
        // its own window's kind, so this write-through is host-only.
        if let Some(meta) = state.window_meta(label) {
            let kind_str = match meta.kind {
                crate::state::WindowKind::FullInstance => "full_instance",
                crate::state::WindowKind::Subwindow => "subwindow",
            };
            // `parent_instance_id` is a window LABEL (see `WindowMeta`'s doc
            // comment), not a srv window_id — resolve it through the same
            // label→window_id lookup the opacity/floating-placement
            // write-throughs already use. A `Subwindow` whose parent hasn't
            // registered its own window_id yet (a narrow creation-order
            // race) is skipped rather than written with a wrong/missing
            // parent — the same class of bounded gap SPEC_PILLAR1_STEP2
            // already accepted for its own write-throughs.
            let parent_window_id: Option<String> = match &meta.parent_instance_id {
                Some(parent_label) => match state.backend_window_id(parent_label) {
                    Some(id) => Some(id),
                    None => {
                        tracing::debug!(
                            label = %label,
                            parent_label = %parent_label,
                            "[window-topology] parent's backend_window_id not yet known — skipping topology write-through"
                        );
                        return serde_json::Value::Null;
                    }
                },
                None => None,
            };
            let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
            let auth_key = state.auth_key.lock().clone();
            let window_id_owned = window_id.to_string();
            let kind_owned = kind_str.to_string();
            std::thread::spawn(move || {
                crate::client::backend_set_window_topology(
                    &web_endpoint,
                    &auth_key,
                    &window_id_owned,
                    &kind_owned,
                    parent_window_id.as_deref(),
                );
            });
        } else {
            tracing::debug!(
                label = %label,
                "[window-topology] no WindowMeta for label — skipping topology write-through (e.g. pool/browser-pane label)"
            );
        }

        // SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P0, PR #2017,
        // 2026-07-08) — this is the trigger point for the slow-path
        // reproject, not "main"'s native browser registration
        // (`on_after_created`). `reproject_from_srv` needs "main"'s own
        // confirmed srv `window_id` to know which `Client.windowids` entry
        // to exclude/treat as the parent-linkage sentinel — that's exactly
        // this call's `window_id` argument, known with certainty here
        // (unlike the original design's `windowids[0]` positional guess,
        // which reagent correctly flagged as broken by `focus_window`
        // reordering — see `ui_thread_gate.pending_slow_path`'s doc
        // comment). Check-and-clear under the gate's lock so this and a
        // late-arriving fast-path snapshot (`launcher_ipc.rs`'s
        // `Event::Snapshot` arm) can't both fire.
        if label == "main" {
            let should_run_slow_path = state.ui_thread_gate.lock().on_main_backend_window_registered();
            if should_run_slow_path {
                tracing::info!(
                    target: "reproject",
                    main_window_id = %window_id,
                    "[reproject] \"main\"'s own backend window ID is now known — running the slow path"
                );
                crate::commands::window::reproject_from_srv(state, window_id.to_string());
            }
        }
    } else {
        tracing::warn!(label = %label, "[window] register_backend_window called with empty window_id — skipped");
    }
    serde_json::Value::Null
}

/// Toggle DevTools for the main window.
///
/// Uses CEF's native show_dev_tools() API, which triggers
/// BrowserViewDelegate::on_popup_browser_view_created with is_devtools=1.
/// That callback creates a top-level CefWindow with a native title bar,
/// producing a standalone DevTools window — identical to Tauri's open_devtools().
pub fn toggle_devtools(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_show_dev_tools(state, label);
    Ok(serde_json::Value::Null)
}

/// Open DevTools focused on the element at the given window-relative
/// coordinates. Equivalent to Chrome's right-click → Inspect Element.
/// Used by the pane context menu's Inspect entry.
pub fn inspect_element_at(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::ui_tasks::post_inspect_element_at(state, label, x, y);
    Ok(serde_json::Value::Null)
}
