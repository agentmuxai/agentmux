// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window creation handlers + frontend-URL resolution for the CEF host —
// open_new_window / open_subwindow, the dev-Vite-port + host-runtime-dir
// resolution that feeds the loaded URL, and the FrontendUrlError surface.
//
// Final carve of the commands/window.rs modularization (Plan 1); with it,
// `mod.rs` becomes a pure re-export shim. Pure move — no behavior change.
//
// `open_new_window` / `open_subwindow` are `pub` (dispatched by ipc.rs).
// `resolve_frontend_base_url` / `assets_missing_data_url` are `pub(crate)`
// (client / drag / window_pool / floating_pane resolve them as
// `commands::window::<name>`, so `mod.rs` re-exports them explicitly).

use std::path::PathBuf;
use std::sync::Arc;

use crate::client::helpers::{html_escape, js_string_literal};
use crate::state::AppState;

// The secondary-window offset helper resolves the current top-level HWND
// (Windows only) from the sibling `lifecycle` module.
#[cfg(target_os = "windows")]
use super::lifecycle::find_own_top_level_window;

/// Resolve the dev Vite port. Honors `AGENTMUX_VITE_PORT` (set by
/// `Taskfile.yml`'s `dev:serve` task when the per-clone deterministic
/// port differs from 5173); falls back to 5173 otherwise. Without this,
/// every child window (pool warmups, tab tear-off, floating pane) loads
/// `localhost:5173` and hits `ERR_CONNECTION_REFUSED` on any other port —
/// only the main window survives because the launcher passes `--url=…`
/// on the CLI. See `docs/analysis/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md`.
fn dev_vite_port() -> u16 {
    std::env::var("AGENTMUX_VITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5173)
}

/// Why `resolve_frontend_base_url` could not return a usable URL.
///
/// Returned by `resolve_frontend_base_url` instead of the old silent
/// fallback to `http://localhost:<vite_port>`, which masked broken
/// installs as renderer-crash loops in production (see
/// `docs/retro/retro-portable-rm-running-install-2026-05-28.md`).
#[derive(Debug)]
pub(crate) enum FrontendUrlError {
    /// `std::env::current_exe()` failed. Extraordinarily rare; on
    /// Windows it would mean we couldn't even resolve our own module
    /// path, which usually only happens for truly corrupted installs.
    ExeUnresolvable(std::io::Error),
    /// We resolved a production install dir but `frontend/index.html`
    /// is not next to the exe. Either the bundle was never built, was
    /// deleted (e.g. by an external `rm -rf` of a running portable —
    /// see retro), or the install layout is otherwise wrong.
    AssetsMissing { checked_path: PathBuf },
}

impl std::fmt::Display for FrontendUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExeUnresolvable(e) => write!(f, "could not resolve current_exe: {}", e),
            Self::AssetsMissing { checked_path } => {
                write!(f, "frontend assets missing at {}", checked_path.display())
            }
        }
    }
}

impl std::error::Error for FrontendUrlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ExeUnresolvable(e) => Some(e),
            Self::AssetsMissing { .. } => None,
        }
    }
}

/// Resolve the base URL for the frontend.
///
/// - **Production:** IPC server serves static files from `frontend/`
///   next to the exe → returns `http://127.0.0.1:<ipc_port>`.
/// - **Dev:** Vite dev server at
///   `http://localhost:<AGENTMUX_VITE_PORT or 5173>`.
/// - **Production with missing assets:** returns
///   `Err(FrontendUrlError::AssetsMissing)`. Historically this fell
///   through to `http://localhost:<vite_port>`, which in production
///   points at nothing and produced silent renderer crash loops
///   (issue #1117 / retro 2026-05-28). Callers should now translate
///   the error into a built-in static error page via
///   `assets_missing_data_url` rather than navigating to a network
///   URL.
pub(crate) fn resolve_frontend_base_url(ipc_port: u16) -> Result<String, FrontendUrlError> {
    // Detect dev mode by the host exe PATH (`is_dev_self`), NOT
    // `AGENTMUX_RUNTIME_MODE`. A running `task dev` AgentMux leaks that env
    // into descendants, so an env-first check would route a *packaged*
    // build's secondary windows (new window, window pool, tear-off, floating
    // pane) to the inherited dev Vite server — they return here before the
    // bundled-asset check below, so there's no `has_frontend` safety net.
    // Build identity is a property of the binary on disk: `task dev` (with or
    // without the launcher) runs the host from `dist/cef-dev/` → Dev; a
    // packaged host → not Dev, regardless of any inherited env.
    if agentmux_common::is_dev_self() {
        return Ok(format!("http://localhost:{}", dev_vite_port()));
    }
    let runtime_dir = resolve_host_runtime_dir()?;
    let index = runtime_dir.join("frontend").join("index.html");
    if index.exists() {
        Ok(format!("http://127.0.0.1:{}", ipc_port))
    } else {
        Err(FrontendUrlError::AssetsMissing { checked_path: index })
    }
}

/// Resolve the directory that contains this host build's runtime
/// assets (`frontend/`, the CEF DLLs, locales, etc.).
///
/// Preference order:
///
/// 1. **`AGENTMUX_HOME` env var** — set by `agentmux-launcher` from
///    *its* resolved `real_exe` path at host-spawn time. This is the
///    authoritative anchor: the launcher always finds the runtime
///    dir by walking from its own current_exe, so the path it
///    exports always points at the on-disk files that actually
///    exist, even if those files were moved or unlinked after host
///    process startup. See
///    `docs/retro/retro-portable-rm-running-install-2026-05-28.md`
///    for why current_exe() is not safe to use directly on Windows.
/// 2. **Fallback to `std::env::current_exe().parent()`** — used in
///    dev mode (`task dev` without the launcher) and any other
///    invocation where AGENTMUX_HOME isn't set. Carries the old
///    Windows-rename hazard, but those invocations don't survive
///    a directory move in the same way (dev mode is short-lived).
fn resolve_host_runtime_dir() -> Result<PathBuf, FrontendUrlError> {
    if let Some(val) = std::env::var_os("AGENTMUX_HOME") {
        let path = PathBuf::from(val);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    let exe = std::env::current_exe().map_err(FrontendUrlError::ExeUnresolvable)?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| FrontendUrlError::AssetsMissing { checked_path: exe })
}

/// Build a self-contained `data:` URL that renders a static
/// "AgentMux install is broken" error page. Used by every caller of
/// `resolve_frontend_base_url` as the navigation target when the
/// resolver returns `Err` — instead of falling back to a network URL
/// that almost certainly points at nothing in production.
///
/// The page contains no auto-reload and no link back into the broken
/// install, so navigating to it can never trigger the crash loop the
/// old silent fallback produced. Only buttons are "Quit" and "Copy
/// path" (the missing asset path is shown so the user knows what to
/// reinstall).
pub(crate) fn assets_missing_data_url(err: &FrontendUrlError) -> String {
    let html = assets_missing_html(err);
    let b64 = cef::base64_encode(Some(html.as_bytes()));
    let b64_str = cef::CefString::from(&b64).to_string();
    format!("data:text/html;base64,{}", b64_str)
}

/// HTML for the broken-install error page. Split out from
/// `assets_missing_data_url` so the button wiring is unit-testable without
/// CEF / base64 round-tripping.
fn assets_missing_html(err: &FrontendUrlError) -> String {
    let (reason, detail) = match err {
        FrontendUrlError::ExeUnresolvable(e) => (
            "Could not determine the install directory".to_string(),
            e.to_string(),
        ),
        FrontendUrlError::AssetsMissing { checked_path } => (
            "AgentMux frontend assets are missing".to_string(),
            format!("Expected at: {}", checked_path.display()),
        ),
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Install broken</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f38ba8; font-size: 22px; margin: 0 0 6px 0; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
.detail code {{ display: inline-block; background: #313244; color: #f9e2af;
               padding: 6px 10px; border-radius: 4px; font-size: 12px;
               font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace;
               word-break: break-all; text-align: left; max-width: 100%; }}
.actions {{ display: flex; gap: 10px; justify-content: center; margin-top: 24px; flex-wrap: wrap; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">⚠</div>
<h1>AgentMux install is broken</h1>
<p>{reason_safe}.</p>
<p class="detail"><code>{detail_safe}</code></p>
<p>Reinstall AgentMux from a fresh portable ZIP to recover. Your saved
sessions and agent state are in <code>~/.agentmux/</code> and are unaffected.</p>
<div class="actions">
<button id="amx-copy">Copy path</button>
<button id="amx-quit">Quit</button>
</div></div>
<script>
(function(){{
  var c = document.getElementById('amx-copy');
  if (c) c.addEventListener('click', function () {{
    if (navigator.clipboard) navigator.clipboard.writeText({detail_js});
  }});
  var q = document.getElementById('amx-quit');
  if (q) q.addEventListener('click', function () {{ window.close(); }});
}})();
</script>
</body></html>"#,
        reason_safe = html_escape(&reason),
        detail_safe = html_escape(&detail),
        detail_js = js_string_literal(&detail),
    )
}

/// Open a new full AgentMux instance (status-bar version click, Ctrl+Shift+N,
/// second `agentmux.exe` launch). Independent top-level window, own taskbar
/// entry, independent lifecycle. See
/// `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md`.
pub fn open_new_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let initial_view = args
        .get("initial_view")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // Opaque JSON string carrying the full blockdef.meta; threaded through pool
    // event payloads and cold-path URL so the new window can call pane.open
    // with the complete meta rather than re-deriving it from the view name.
    let initial_meta = args
        .get("initial_meta")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // H.7 invariant — also enforced inside open_window_with_kind (cold path).
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_new_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    // Pool-first path — instant show with no renderer-spawn latency (~3s saved).
    // On macOS/Linux, emits pool:new-window (no workspaceId → fresh workspace).
    // On Windows, delegates to promote_pool_window with workspace_id="" and physical-pixel
    // anchor (bypassing DIP→physical DPI double-conversion; see promote_pool_window_for_new_window).
    let (pos_x, pos_y) = get_offset_position();
    let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);
    if let Some(label) = crate::commands::window_pool::promote_pool_window_for_new_window(
        state, pos_x, pos_y, win_w, win_h, initial_view.clone(), initial_meta.clone(),
    ) {
        tracing::info!(
            target: "pool:new-window",
            label = %label,
            "[pool:new-window] served from pool — skipping cold-path window creation"
        );
        return Ok(serde_json::json!(label));
    }

    // Cold path — spin up a fresh CEF window (~2.5–3.5 s).
    open_window_with_kind(
        state,
        crate::state::WindowKind::FullInstance,
        None,
        initial_view.as_deref(),
        initial_meta.as_deref(),
        None,
    )
}

/// Open a sub-window tied to `parent_instance_id`. **Not exposed to users** —
/// reserved for agent / backend callers that need a transient auxiliary
/// top-level window (tool-spawned panels, diff views, etc.). Sub-windows are
/// hidden from the taskbar via `ITaskbarList::DeleteTab` and close when their
/// parent full instance closes.
pub fn open_subwindow(
    state: &Arc<AppState>,
    parent_instance_id: String,
) -> Result<serde_json::Value, String> {
    // Reject if the parent isn't a known live FullInstance — prevents
    // orphan sub-windows and enforces the lifecycle rule in the spec.
    //
    // Phase B.5 (window_meta step d, refined twice):
    // * Round-1 fix used `state.window_meta()` (shadow-first), which
    //   covered the task-dev-mode regression but allowed a NEW orphan
    //   bug: shadow lags on close, so during the gap between host's
    //   sync `on_before_close` removal and the launcher's async
    //   `WindowClosed` event arrival, this check could still see a
    //   already-closing parent. (codex P2 PR #592 round-2.)
    // * Refined: read host_meta DIRECTLY for this liveness check.
    //   Host_meta is synchronously written in on_after_created and
    //   removed in on_before_close (per the round-2 step-d
    //   refinement keeping host_meta as a sync cache), so it
    //   correctly reflects "is the parent currently open" without
    //   shadow's async lag. Works in `task dev` mode too (host_meta
    //   populated by on_after_created regardless of launcher
    //   presence).
    let parent_ok = state
        .window_meta
        .lock()
        .get(&parent_instance_id)
        .map(|m| m.kind == crate::state::WindowKind::FullInstance)
        .unwrap_or(false);
    if !parent_ok {
        return Err(format!(
            "open_subwindow: unknown or non-full-instance parent label={parent_instance_id}"
        ));
    }
    open_window_with_kind(
        state,
        crate::state::WindowKind::Subwindow,
        Some(parent_instance_id),
        None,
        None,
        None,
    )
}

/// SPEC_PILLAR1_STEP4 Phase 2 — `pub(crate)` (was private) so the reproject
/// driver (`launcher_ipc::reproject_from_snapshot`) can call it directly,
/// bypassing the IPC-handler wrappers (`open_new_window`/`open_subwindow`)
/// that assume a live, frontend-originated request.
///
/// `explicit_rect`: when `Some`, skips `get_offset_position`/
/// `get_secondary_window_size`'s offset/70%-of-monitor placement heuristic
/// and uses the given rect verbatim — the reproject driver's fast path
/// passes the launcher snapshot's `last_rect` here so a recreated window
/// lands roughly where it was, instead of at a new-window default position.
/// `None` preserves today's behavior exactly (both existing callers pass
/// `None`).
pub(crate) fn open_window_with_kind(
    state: &Arc<AppState>,
    kind: crate::state::WindowKind,
    parent_instance_id: Option<String>,
    initial_view: Option<&str>,
    initial_meta: Option<&str>,
    explicit_rect: Option<agentmux_common::ipc::Rect>,
) -> Result<serde_json::Value, String> {
    // PR #6 H.7 — refuse top-level creation while any pane is mid-close.
    // See `SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` and the smoke retro
    // at `docs/retro/smoke-test-0.33.586-and-pr5-plan-2026-05-02.md`:
    // creating a top-level CEF window while a pane is in `Closing` hits
    // a Chromium v146 deadlock (HiddenSinceOpen + IPC backpressure)
    // that wedges the message loop. Frontend should retry on next tick.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let url = match resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let view_param = initial_view
                .filter(|v| !v.is_empty())
                .map(|v| format!("&initialView={}", v))
                .unwrap_or_default();
            let meta_param = initial_meta
                .filter(|m| !m.is_empty())
                .map(|m| format!("&initialMeta={}", percent_encode(m)))
                .unwrap_or_default();
            format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}{}{}",
                base_url, separator, ipc_port, ipc_token, label, view_param, meta_param
            )
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                label = %label,
                "[window] frontend assets unavailable — opening static error page (the new window will display an 'install broken' notice instead of crash-looping)",
            );
            assets_missing_data_url(&e)
        }
    };

    tracing::info!(label = %label, kind = ?kind, parent = ?parent_instance_id, "[window] open window");

    // Phase B.5 (window_meta step d) — push the pre-create handoff
    // (label + kind + parent). Replaces the previous parallel
    // `window_meta.insert` + `pending_window_labels.push` pair.
    let (pos_x, pos_y, win_w, win_h) = match explicit_rect {
        Some(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
        None => {
            let (pos_x, pos_y) = get_offset_position();
            let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);
            (pos_x, pos_y, win_w, win_h)
        }
    };

    // Phase F.1 — routed through the host reducer.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind,
                parent_instance_id,
            },
        },
    );

    // Post to CEF UI thread — window_create_top_level must run there.
    // true = frameless: secondary app windows use the same custom title bar as main.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
        true,
    );

    // Phase B.7.3.3 — typed launcher events drive InstancePanel
    // atoms via the CEF JS bridge; no sync emit here.

    Ok(serde_json::json!(label))
}

/// SPEC_PILLAR1_STEP4 Phase 2 — the fast-path reproject driver. Called once
/// per process life from `launcher_ipc::apply_event_to_shadow`'s
/// `Event::Snapshot` arm, with the launcher's live window list (survives a
/// host-only crash — see the spec for why this is preferred over srv's
/// durable-but-poorer-fidelity `Client.windowids`).
///
/// Idempotent by construction, not by an explicit "have we already
/// reprojected" flag: `"main"` is always filtered out (it's created by the
/// separate, unconditional cold-start path regardless of what the snapshot
/// says — recreating it here would double-create). On a genuine first-ever
/// launch the snapshot has zero entries beyond (at most) a stale `"main"`,
/// so this call does nothing.
///
/// Ordering: `FullInstance` windows are recreated before `Subwindow`s, so a
/// subwindow's parent always exists (under its NEW label) by the time the
/// subwindow needs it. This is a stable sort by kind, not a dependency
/// graph — sufficient because every real subwindow's parent is a
/// `FullInstance` (per `WindowKind`'s own doc comment; there is no
/// subwindow-of-subwindow nesting in this codebase).
///
/// Old-label→new-label remapping is mandatory, not optional: every
/// recreated window gets a fresh `window-<uuid>` label
/// (`open_window_with_kind`), so a subwindow's `parent_label` from the
/// snapshot (an OLD label) must be translated to the parent's NEW label
/// before being passed to `open_window_with_kind`. `"main"` is seeded into
/// the remap table as `"main"` → `"main"` since its label is stable across
/// restarts (never regenerated).
pub(crate) fn reproject_from_snapshot(
    state: &Arc<AppState>,
    windows: &[agentmux_common::ipc::WindowSnapshot],
) {
    use std::collections::HashMap;

    let mut to_create: Vec<&agentmux_common::ipc::WindowSnapshot> =
        windows.iter().filter(|w| w.label != "main").collect();
    if to_create.is_empty() {
        tracing::debug!(target: "reproject", "[reproject] nothing to recreate beyond main");
        return;
    }
    // FullInstance before Subwindow — stable sort preserves the snapshot's
    // own relative ordering within each kind. `WindowSnapshot.kind` is the
    // WIRE type (`agentmux_common::ipc::WindowKind`), distinct from the
    // host's own `crate::state::WindowKind` — mapped one-to-one, same
    // conversion `apply_shadow_projection`'s `Event::WindowOpened` arm
    // already does.
    let to_host_kind = |k: agentmux_common::ipc::WindowKind| match k {
        agentmux_common::ipc::WindowKind::FullInstance => crate::state::WindowKind::FullInstance,
        agentmux_common::ipc::WindowKind::Subwindow => crate::state::WindowKind::Subwindow,
    };
    to_create.sort_by_key(|w| match w.kind {
        agentmux_common::ipc::WindowKind::FullInstance => 0,
        agentmux_common::ipc::WindowKind::Subwindow => 1,
    });

    let mut label_remap: HashMap<String, String> = HashMap::new();
    label_remap.insert("main".to_string(), "main".to_string());

    let mut created = 0usize;
    let mut skipped = 0usize;
    for w in to_create {
        let new_parent = match &w.parent_label {
            Some(old_parent) => match label_remap.get(old_parent) {
                Some(new_label) => Some(new_label.clone()),
                None => {
                    // Parent wasn't (re)created — e.g. it was itself
                    // skipped, or the snapshot listed a Subwindow before
                    // its FullInstance parent existed at all (a crash
                    // mid-creation). Skip rather than create an orphan
                    // with a dangling parent link.
                    tracing::warn!(
                        target: "reproject",
                        label = %w.label,
                        parent_label = %old_parent,
                        "[reproject] parent not found in remap table — skipping this window"
                    );
                    skipped += 1;
                    continue;
                }
            },
            None => None,
        };
        match open_window_with_kind(state, to_host_kind(w.kind), new_parent, None, None, w.last_rect) {
            Ok(new_label_val) => {
                let new_label = new_label_val.as_str().unwrap_or_default().to_string();
                tracing::info!(
                    target: "reproject",
                    old_label = %w.label,
                    new_label = %new_label,
                    kind = ?w.kind,
                    parent_label = ?w.parent_label,
                    had_rect = w.last_rect.is_some(),
                    "[reproject] recreated window"
                );
                label_remap.insert(w.label.clone(), new_label);
                created += 1;
            }
            Err(e) => {
                tracing::warn!(
                    target: "reproject",
                    label = %w.label,
                    error = %e,
                    "[reproject] failed to recreate window"
                );
                skipped += 1;
            }
        }
    }
    tracing::info!(
        target: "reproject",
        created,
        skipped,
        "[reproject] fast-path reproject complete"
    );
}

/// SPEC_PILLAR1_STEP4 Phase 3 — the slow-path reproject driver. Called once
/// per process life from `commands/window/meta.rs::register_backend_window`'s
/// `"main"` branch, only when no fast-path (launcher-snapshot) data was
/// available — e.g. a full process-tree kill (launcher died too, so it has
/// no in-memory history either) or `task dev` standalone mode (no launcher
/// at all). Reads srv's durable `Client.windowids` + each window's `kind`/
/// `parent_window_id` (SPEC_PILLAR1_STEP3) instead.
///
/// `main_window_id` is `"main"`'s own confirmed srv `window_id`, passed in
/// by the caller — NOT derived positionally from `Client.windowids[0]`.
/// reagent (P0, PR #2017, 2026-07-08) caught that the earlier design did
/// exactly that, on the assumption index 0 was reliably `"main"`; it isn't —
/// `focus_window` (`agentmux-srv/.../wcore/window.rs:164`) reorders
/// `Client.windowids` to put the last-focused window at index 0 on every
/// focus change, so index 0 is "whichever window the user looked at last,"
/// not a stable identity. Filtering `main_window_id` out BY VALUE (wherever
/// it appears in the list, or not at all) is correct regardless of
/// reordering; the caller has it with certainty because this only runs from
/// `register_backend_window`'s own `"main"` branch, i.e. after `"main"`
/// itself resolved and confirmed that id.
///
/// No `last_rect` is available this way — Step 2/3 never persisted window
/// position/size (see the spec's §4 risk) — so every recreated window lands
/// at `open_window_with_kind`'s default offset placement, not where the
/// user left it. Kind/parent/content are still fully correct.
///
/// Does its network I/O on a spawned thread, never the calling (UI) thread —
/// `backend_get_client_window_ids`/`backend_get_window_topology` are
/// blocking calls (same raw-TCP shape as every other `backend_*` read/write
/// helper in this codebase). By the time this runs, `ui_thread_gate.ready`
/// is already true (this fires well after "main"'s own registration), so
/// `open_window_with_kind` posting from this background thread is safe —
/// the same already-verified-safe pattern as any pool-window creation
/// posted after `"main"` registers.
///
/// Converges on the exact same per-window recreation code
/// (`reproject_from_snapshot`) the fast path uses, per the parent design
/// doc's "both tiers converge on the same per-window recreation code path."
pub(crate) fn reproject_from_srv(state: &Arc<AppState>, main_window_id: String) {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let Some(window_ids) = crate::client::backend_get_client_window_ids(&web_endpoint, &auth_key) else {
            tracing::warn!(target: "reproject", "[reproject] slow path: could not read Client.windowids from srv");
            return;
        };

        // reagent P1 (PR #2017) — `reproject_from_snapshot`'s `label_remap` is
        // seeded only with the string `"main"` → `"main"` (the host's own
        // stable label for it), not srv's UUID for it. A subwindow whose
        // persisted `parent_window_id` equals `main_window_id` must be
        // translated to the `"main"` sentinel here — otherwise the remap
        // lookup finds nothing and the subwindow is silently skipped as if
        // its parent were missing.
        let extra_ids: Vec<&String> = window_ids.iter().filter(|id| *id != &main_window_id).collect();
        if extra_ids.is_empty() {
            tracing::debug!(
                target: "reproject",
                window_count = window_ids.len(),
                "[reproject] slow path: nothing beyond main to recreate"
            );
            return;
        }

        let mut snapshots: Vec<agentmux_common::ipc::WindowSnapshot> = Vec::new();
        for window_id in extra_ids {
            let Some((kind, parent_window_id)) = crate::client::backend_get_window_topology(&web_endpoint, &auth_key, window_id) else {
                tracing::warn!(target: "reproject", window_id = %window_id, "[reproject] slow path: GetWindow failed — skipping this window");
                continue;
            };
            let kind = match kind.as_deref() {
                Some("subwindow") => agentmux_common::ipc::WindowKind::Subwindow,
                Some("full_instance") => agentmux_common::ipc::WindowKind::FullInstance,
                Some(other) => {
                    tracing::warn!(target: "reproject", window_id = %window_id, kind = %other, "[reproject] slow path: unrecognized persisted kind — defaulting to FullInstance");
                    agentmux_common::ipc::WindowKind::FullInstance
                }
                None => {
                    tracing::warn!(target: "reproject", window_id = %window_id, "[reproject] slow path: no persisted kind (pre-Step-3 window row?) — defaulting to FullInstance");
                    agentmux_common::ipc::WindowKind::FullInstance
                }
            };
            let parent_label = parent_window_id.map(|pid| {
                if pid == main_window_id {
                    "main".to_string()
                } else {
                    pid
                }
            });
            snapshots.push(agentmux_common::ipc::WindowSnapshot {
                label: window_id.clone(),
                kind,
                parent_label,
                hwnd: None,
                visible: true,
                iconic: false,
                last_rect: None,
                foregrounded_since_open: true,
            });
        }
        tracing::info!(
            target: "reproject",
            window_count = snapshots.len(),
            "[reproject] slow path: recreating windows from srv"
        );
        reproject_from_snapshot(&state, &snapshots);
    });
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b => { let _ = std::fmt::Write::write_fmt(&mut out, format_args!("%{:02X}", b)); }
        }
    }
    out
}

/// Get an offset position for a new window: 30px right and 30px down from the current window.
fn get_offset_position() -> (i32, i32) {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return (rect.left + 30, rect.top + 30);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT;
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }
    #[cfg(not(target_os = "windows"))]
    (100, 100)
}

/// Compute 70% of the monitor's work area for a secondary window at (px, py).
/// Falls back to 1200x800 if the monitor can't be determined.
fn get_secondary_window_size(px: i32, py: i32) -> (i32, i32) {
    #[cfg(target_os = "windows")]
    {
        use crate::app::get_monitor_work_area;
        if let Some((_x, _y, work_w, work_h)) = get_monitor_work_area(px, py) {
            return ((work_w as f64 * 0.70) as i32, (work_h as f64 * 0.70) as i32);
        }
    }
    (1200, 800)
}

#[cfg(test)]
mod assets_missing_tests {
    use super::{assets_missing_html, FrontendUrlError};
    use std::path::PathBuf;

    #[test]
    fn copy_path_button_has_working_handler() {
        // Regression: js_string_literal() returns a DOUBLE-quoted JS string;
        // dropping it into a double-quoted onclick="..." attribute terminated
        // the attribute early, leaving the "Copy path" button inert. The
        // handler must be wired in a <script> block instead.
        let err = FrontendUrlError::AssetsMissing {
            checked_path: PathBuf::from("MISSING_ASSETS"),
        };
        let html = assets_missing_html(&err);

        assert!(
            !html.contains("onclick=\"navigator.clipboard"),
            "Copy path must not use an inline onclick with a double-quoted literal"
        );
        assert!(html.contains("addEventListener"), "handler should use addEventListener");
        assert!(html.contains("id=\"amx-copy\""), "Copy button needs its id");
        // The detail string is emitted as a valid JS string literal in script
        // context (js_string_literal wraps in double quotes).
        assert!(
            html.contains("writeText(\"Expected at: MISSING_ASSETS\")"),
            "Copy path must writeText the detail as a valid JS string literal"
        );
    }
}
