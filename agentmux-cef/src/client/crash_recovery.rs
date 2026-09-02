// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RequestHandler crash / auth methods for `AgentMuxHandler` — renderer crash
//! budgets, gated memory-pause recovery, and HTTP-basic auth registry.
//! Extracted verbatim from client/mod.rs.

use cef::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::helpers::{html_escape, js_string_literal};
use super::recovery_pages::{
    crash_loop_terminal_page, memory_paused_page, record_memory_pause, recovery_navigation_url,
};
use super::{AgentMuxHandler, CRASH_BUDGET, CRASH_BUDGET_WINDOW, MEMORY_PAUSE_WINDOW, RESUME_FLOOR_MB};

impl AgentMuxHandler {
    /// Render-process terminated — typically OOM, a renderer-side panic, or
    /// some native bug inside CEF/Chromium. Without this hook the window
    /// just turns white. We log the cause and replace the white page with
    /// a recovery HTML page that offers Reload / Quit buttons.
    ///
    /// See docs/specs/SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).
    pub(crate) fn on_render_process_terminated(
        &mut self,
        browser: Option<&mut Browser>,
        status: TerminationStatus,
        error_code: i32,
        error_string: Option<&CefString>,
    ) {
        let reason = if status == TerminationStatus::PROCESS_OOM {
            "out of memory"
        } else if status == TerminationStatus::PROCESS_CRASHED {
            "renderer process crashed"
        } else if status == TerminationStatus::ABNORMAL_TERMINATION {
            "abnormal termination"
        } else {
            "renderer process terminated"
        };

        let detail = error_string.map(CefString::to_string).unwrap_or_default();
        // Rate-limit the renderer_terminated event on the `crash`
        // target. The crash budget below caps per-browser crashes at
        // CRASH_BUDGET within CRASH_BUDGET_WINDOW, but if many
        // browsers crash simultaneously the aggregate write rate to
        // the host log can still spike. RENDERER_TERMINATED_LOG_MIN_GAP
        // throttles to at most one full log line per 100 ms across
        // the whole process; suppressed events are counted and the
        // count is emitted as a `suppressed` field on the next
        // un-throttled event so no information is lost. See
        // docs/retro/retro-portable-rm-running-install-2026-05-28.md
        // for the original 884 MB / 22 min log spam that motivated
        // both this and the per-browser budget.
        const RENDERER_TERMINATED_LOG_MIN_GAP: Duration = Duration::from_millis(100);
        static LAST_LOGGED_AT_MS: AtomicU64 = AtomicU64::new(0);
        static SUPPRESSED_SINCE: AtomicU64 = AtomicU64::new(0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last_ms = LAST_LOGGED_AT_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < RENDERER_TERMINATED_LOG_MIN_GAP.as_millis() as u64 {
            SUPPRESSED_SINCE.fetch_add(1, Ordering::Relaxed);
        } else {
            let suppressed = SUPPRESSED_SINCE.swap(0, Ordering::Relaxed);
            LAST_LOGGED_AT_MS.store(now_ms, Ordering::Relaxed);
            tracing::error!(
                target: "crash",
                kind = "renderer_terminated",
                reason,
                error_code,
                detail = %detail,
                suppressed_since_last = suppressed,
                "{}", reason,
            );
        }

        // ── Gated renderer recovery (SPEC_GATED_RENDERER_RECOVERY §6.B) ──
        // A renderer OOM while the system commit limit is exhausted is
        // transient OS pressure, NOT a broken renderer — no amount of retrying
        // helps until memory frees, and the standard 3-crashes-in-10s budget
        // would wrongly declare "give up" on a window that is fully recoverable
        // once commit drops. So: if the termination is PROCESS_OOM and
        // commit-free is below RESUME_FLOOR_MB, do NOT consume the crash
        // budget; show a recoverable "low memory" page (state is durable in
        // srv, so Resume re-projects everything). Bounded separately by
        // MEMORY_PAUSE_BUDGET so that *total* exhaustion — where even this tiny
        // page can't render and re-fires the handler — still converges on the
        // give-up page rather than looping. (Auto-resume + a renderer-free
        // native overlay for total exhaustion are Phase 1b.)
        if status == TerminationStatus::PROCESS_OOM {
            let commit_free = crate::memory_heartbeat::commit_free_mb();
            if commit_free < RESUME_FLOOR_MB {
                if let Some(bid) = browser.as_ref().map(|b| b.identifier()) {
                    // Resolve the live app URL up front: if the frontend assets
                    // are unavailable that's a *different* failure (the
                    // 2026-05-28 rm-while-running pattern), so leave the
                    // memory-pause history untouched and fall through to the
                    // normal path, which has the assets-missing handling.
                    if let Ok(base_url) =
                        crate::commands::window::resolve_frontend_base_url(self.resolved_ipc_port())
                    {
                        let now = Instant::now();
                        let hist = self.memory_pause_history.entry(bid).or_default();
                        let within_budget = record_memory_pause(hist, now);
                        let pauses_in_window = hist.len();
                        if within_budget {
                            // Clone to an owned Browser (mirrors on_before_close)
                            // for the label lookup + navigation; the original
                            // `browser` Option stays intact for the fall-through
                            // paths below. as_deref().cloned() borrows, not moves.
                            if let Some(mut owned) = browser.as_deref().cloned() {
                                let app_url = self.recovery_target_url(&mut owned, &base_url);
                                tracing::warn!(
                                    target: "crash",
                                    kind = "renderer_memory_paused",
                                    browser_id = bid,
                                    commit_free_mb = commit_free,
                                    resume_floor_mb = RESUME_FLOOR_MB,
                                    pauses_in_window,
                                    "renderer OOM under low system commit — paused (NOT counted against the crash budget)",
                                );
                                let html = memory_paused_page(reason, error_code, commit_free, &app_url);
                                let b64 = cef::base64_encode(Some(html.as_bytes()));
                                let b64_str = CefString::from(&b64).to_string();
                                let data_uri = format!("data:text/html;base64,{}", b64_str);
                                let uri = CefString::from(data_uri.as_str());
                                if let Some(frame) = owned.main_frame() {
                                    frame.load_url(Some(&uri));
                                }
                                return;
                            }
                            // browser was None (unusual on this path) — fall
                            // through to the normal crash-budget handling below.
                        } else {
                            tracing::error!(
                                target: "crash",
                                kind = "memory_pause_budget_exceeded",
                                browser_id = bid,
                                pauses_in_window,
                                window_secs = MEMORY_PAUSE_WINDOW.as_secs(),
                                "memory-pause budget exceeded (commit totally exhausted) — falling through to the give-up page",
                            );
                            // fall through to the crash-budget path below.
                        }
                    }
                }
            }
        }

        // Crash budget — if this browser has crashed more than
        // CRASH_BUDGET times in CRASH_BUDGET_WINDOW, abandon
        // auto-recovery and load a terminal "give up" page that does
        // NOT call frame.load_url again. This breaks the loop the
        // 2026-05-28 incident produced: a wedged renderer slot meant
        // every recovery-page load itself terminated, re-firing this
        // handler at ~108 events/sec for 22 minutes (139k crashes,
        // 884 MB log). See SPEC_SERVICE_SUPERVISION prime directive
        // ("bounded recovery; never an infinite restart loop") and
        // docs/retro/retro-portable-rm-running-install-2026-05-28.md.
        //
        // Pre-budget-check is cheap and runs on every crash; the work
        // it gates (resolve_frontend_base_url + format! + base64 +
        // load_url) is several orders of magnitude more expensive,
        // so even N crashes within budget incur no measurable
        // overhead from this block.
        let browser_id = browser.as_ref().map(|b| b.identifier());
        if let Some(bid) = browser_id {
            let now = Instant::now();
            let history = self.crash_history.entry(bid).or_default();
            // Prune entries outside the window before counting.
            while history.front().is_some_and(|t| now.duration_since(*t) > CRASH_BUDGET_WINDOW) {
                history.pop_front();
            }
            history.push_back(now);
            if history.len() > CRASH_BUDGET {
                let crashes_in_window = history.len();
                tracing::error!(
                    target: "crash",
                    kind = "crash_loop_aborted",
                    browser_id = bid,
                    crashes_in_window,
                    window_secs = CRASH_BUDGET_WINDOW.as_secs(),
                    "crash budget exceeded — abandoning auto-recovery for this browser",
                );
                let html = crash_loop_terminal_page(reason, error_code, crashes_in_window);
                let b64 = cef::base64_encode(Some(html.as_bytes()));
                let b64_str = CefString::from(&b64).to_string();
                let data_uri = format!("data:text/html;base64,{}", b64_str);
                let uri = CefString::from(data_uri.as_str());
                if let Some(b) = browser {
                    if let Some(frame) = b.main_frame() {
                        frame.load_url(Some(&uri));
                    }
                }
                return;
            }
        }

        // Resolve the real frontend URL so the Reload button can navigate
        // back to the live app instead of reloading the recovery page
        // itself. Matches the format used by
        // commands::window::resolve_frontend_base_url and its callers
        // (see window.rs:400, window.rs:430, drag.rs:294 — all use the
        // same ipc_port / ipc_token query params).
        //
        // If the resolver returns Err (frontend assets missing — the
        // 2026-05-28 incident pattern where an external `rm -rf` of a
        // running portable left current_exe()'s parent dir empty),
        // short-circuit to the "install broken" static page instead of
        // pointing the Reload button at a URL that would itself crash.
        // See docs/retro/retro-portable-rm-running-install-2026-05-28.md.
        // Reload must bring a recovered window back as ITSELF — preserving
        // windowLabel and (for tear-off / floating-pane windows) workspaceId /
        // floatingPaneId. recovery_target_url reuses the window's own pre-crash
        // URL when possible. as_deref().cloned() borrows browser (a clone),
        // leaving the original intact for the load below. (codex P2 #1229.)
        let mut recovery_owned = browser.as_deref().cloned();
        let app_url = match crate::commands::window::resolve_frontend_base_url(self.resolved_ipc_port()) {
            Ok(base_url) => match recovery_owned.as_mut() {
                Some(owned) => self.recovery_target_url(owned, &base_url),
                None => recovery_navigation_url(
                    &base_url,
                    self.resolved_ipc_port(),
                    &self.state.ipc_token,
                    None,
                ),
            },
            Err(e) => {
                tracing::error!(
                    target: "crash",
                    error = %e,
                    "renderer crash recovery: frontend assets unavailable — loading static install-broken page instead of an unresolvable network URL",
                );
                let url = crate::commands::window::assets_missing_data_url(&e);
                if let Some(b) = browser {
                    if let Some(frame) = b.main_frame() {
                        let uri = CefString::from(url.as_str());
                        frame.load_url(Some(&uri));
                    }
                }
                return;
            }
        };

        let detail_block = if detail.is_empty() {
            String::new()
        } else {
            format!("<p class=\"detail\"><code>{}</code></p>", html_escape(&detail))
        };

        // Build the recovery page. Plain HTML+CSS, no JS dependencies
        // beyond a single click handler, so it renders even if the
        // frontend bundle is dead. The Reload button navigates directly
        // to the real app URL (NOT location.reload() — that would just
        // re-render the same data: URL). CEF will spawn a fresh renderer
        // subprocess for the navigation.
        //
        // NOTE on ipc_token exposure: the token is already present in
        // the live app URL that was loaded before the crash (it's in
        // the location bar for the dead renderer's process). Embedding
        // it in the recovery HTML that runs inside the same browser
        // doesn't extend its reach — the HTML is ephemeral, not
        // persisted to disk, and `window.close()` or the next crash
        // clears it.
        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>AgentMux — Recovery</title>
    <style>
        :root {{
            color-scheme: dark;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1e1e2e;
            color: #cdd6f4;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            padding: 24px;
            box-sizing: border-box;
        }}
        .recovery {{
            text-align: center;
            max-width: 540px;
            padding: 36px;
            background: #181825;
            border: 1px solid #313244;
            border-radius: 10px;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
        }}
        .icon {{
            font-size: 36px;
            line-height: 1;
            margin-bottom: 12px;
        }}
        h1 {{
            color: #f9e2af;
            font-size: 22px;
            margin: 0 0 6px 0;
        }}
        .reason {{
            color: #a6adc8;
            font-size: 14px;
            margin: 0 0 20px 0;
            font-style: italic;
        }}
        p {{
            color: #bac2de;
            line-height: 1.55;
            margin: 0 0 12px 0;
            font-size: 14px;
        }}
        .detail code {{
            display: inline-block;
            background: #313244;
            color: #f38ba8;
            padding: 6px 10px;
            border-radius: 4px;
            font-size: 12px;
            font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace;
            word-break: break-all;
            text-align: left;
            max-width: 100%;
        }}
        .actions {{
            display: flex;
            gap: 10px;
            justify-content: center;
            margin-top: 24px;
            flex-wrap: wrap;
        }}
        button {{
            padding: 10px 22px;
            border: 1px solid #45475a;
            border-radius: 6px;
            background: #313244;
            color: #cdd6f4;
            cursor: pointer;
            font-size: 13px;
            font-family: inherit;
            transition: background 0.1s, border-color 0.1s;
        }}
        button:hover {{
            background: #45475a;
            border-color: #585b70;
        }}
        button.primary {{
            background: #89b4fa;
            color: #1e1e2e;
            border-color: #89b4fa;
            font-weight: 600;
        }}
        button.primary:hover {{
            background: #74a0f8;
            border-color: #74a0f8;
        }}
        .footer {{
            color: #6c7086;
            font-size: 11px;
            margin-top: 18px;
            font-family: ui-monospace, monospace;
        }}
    </style>
</head>
<body>
    <div class="recovery" role="alertdialog" aria-labelledby="title">
        <div class="icon">⚠</div>
        <h1 id="title">AgentMux hit a problem</h1>
        <p class="reason">Reason: {reason_safe}</p>
        {detail_block}
        <p>Your open sessions are saved on disk. Reloading will bring you back where you left off.</p>
        <div class="actions">
            <button class="primary" id="reload-btn">Reload window</button>
            <button onclick="window.close()">Quit</button>
        </div>
        <div class="footer">error_code={error_code}</div>
    </div>
    <script>
        // The Reload button navigates to the live app URL (not
        // location.reload, which would just re-render this data: page).
        // The URL is injected by the host at HTML-build time.
        document.getElementById('reload-btn').addEventListener('click', function() {{
            location.href = {app_url_js};
        }});
    </script>
</body>
</html>"#,
            reason_safe = html_escape(reason),
            detail_block = detail_block,
            error_code = error_code,
            app_url_js = js_string_literal(&app_url),
        );

        // Load the recovery page in the main frame of the dead browser.
        // The renderer subprocess will be re-spawned by CEF when we
        // navigate, so the new page mounts in a fresh process.
        if let Some(b) = browser {
            if let Some(frame) = b.main_frame() {
                let b64 = cef::base64_encode(Some(html.as_bytes()));
                let b64_str = CefString::from(&b64).to_string();
                let data_uri = format!("data:text/html;base64,{}", b64_str);
                let uri = CefString::from(data_uri.as_str());
                frame.load_url(Some(&uri));
            }
        }
    }

    /// CEF asks the embedder for HTTP Basic / Digest credentials on a
    /// 401/407. Browser-pane requests get surfaced to the renderer via
    /// `browser-pane-auth-required` so the user can type credentials;
    /// non-browser-pane requests (the main host window's frontend
    /// load) are declined — those shouldn't hit auth-challenged
    /// resources, and silently failing matches the prior behavior.
    ///
    /// Returns 1 (async — we'll resolve the callback via
    /// `browser_pane_auth_submit` / `browser_pane_auth_cancel`) or 0
    /// (sync no — CEF aborts the request).
    ///
    /// Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
    pub(crate) fn on_auth_credentials(
        &mut self,
        browser: Option<&mut Browser>,
        origin_url: Option<&CefString>,
        is_proxy: ::std::os::raw::c_int,
        host: Option<&CefString>,
        port: ::std::os::raw::c_int,
        realm: Option<&CefString>,
        _scheme: Option<&CefString>,
        callback: Option<&mut AuthCallback>,
    ) -> ::std::os::raw::c_int {
        // [DIAG] Top-of-function entry log — fires unconditionally before
        // every early-return so we can confirm whether CEF is invoking
        // the callback at all for a given URL. The reagent-merged
        // browser-pane auth flow (#906) appeared to fail silently for
        // some sites in dev mode (e.g. https://pulse.asaf.cc returns
        // ERR_INVALID_AUTH_CREDENTIALS without `[browser-pane-auth]`
        // ever logging). This entry log narrows the diagnosis:
        //   - Visible → CEF is calling the callback; early return below
        //     OR downstream path is the problem.
        //   - Not visible → CEF is suppressing the call entirely
        //     (caching, security policy, missing vtable wire-up).
        // Logs `origin/host/port/realm/is_proxy/has_browser/has_callback`
        // so all the discriminators are captured even on the silent-
        // decline branches that follow.
        let origin_dbg = origin_url.map(CefString::to_string).unwrap_or_default();
        let host_dbg = host.map(CefString::to_string).unwrap_or_default();
        let realm_dbg = realm.map(CefString::to_string).unwrap_or_default();
        tracing::info!(
            "[browser-pane-auth][ENTRY] origin={:?} host={:?}:{} realm={:?} \
             is_proxy={} has_browser={} has_callback={}",
            origin_dbg,
            host_dbg,
            port,
            realm_dbg,
            is_proxy != 0,
            browser.is_some(),
            callback.is_some(),
        );

        // Resolve the pane block_id from the browser ref. If this isn't
        // a browser-pane browser (i.e. it's the host frontend's browser),
        // we have no UI to prompt — decline and let CEF fail the
        // request. The host frontend should never hit auth challenges.
        let Some(b) = browser.as_deref() else {
            tracing::warn!("[browser-pane-auth] no browser ref — declining");
            return 0;
        };
        let Some(block_id) =
            crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
        else {
            tracing::info!(
                "[browser-pane-auth] not a browser pane (host frontend?) — declining"
            );
            return 0;
        };
        let Some(cb) = callback else {
            // [DIAG] Previously silent. If we reach here CEF gave us a
            // browser + a resolvable pane block_id but no callback —
            // an unusual combination worth logging so the diagnosis
            // path doesn't have a blind spot.
            tracing::warn!(
                "[browser-pane-auth] callback is None (block={}) — declining",
                block_id,
            );
            return 0;
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let origin = origin_url.map(CefString::to_string).unwrap_or_default();
        let host_str = host.map(CefString::to_string).unwrap_or_default();
        let realm_str = realm.map(CefString::to_string).unwrap_or_default();
        let is_proxy_bool = is_proxy != 0;

        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane-auth][{}] auth-required origin={:?} host={:?}:{} realm={:?} is_proxy={} request_id={}",
            block_id_short, origin, host_str, port, realm_str, is_proxy_bool, request_id,
        );

        // Park the callback so the renderer's submit/cancel IPC can
        // resolve it. The callback IS reference-counted internally
        // (RefGuard) — `cb.clone()` bumps the refcount so the registry
        // can hold it after CEF's invocation returns. Pass block_id so
        // `cancel_for_block` can clean up when the pane closes.
        crate::browser_pane::auth::register(
            request_id.clone(),
            block_id.clone(),
            cb.clone(),
        );

        // Hand off to the credential broker: identity-resolve → stored-
        // credential lookup → either a human-approval subwindow (never a
        // `<Modal>` — see `credential_broker`'s own doc comment for why)
        // or, on any friction at all (no bound identity, no stored
        // credential, srv unreachable, etc.), the exact same
        // `browser-pane-auth-required` broadcast this function emitted
        // unconditionally before this feature existed. See
        // `docs/status/majestic-painting-minsky` plan.
        crate::credential_broker::on_auth_challenge(
            self.state.clone(),
            request_id,
            block_id,
            origin,
            host_str,
            port,
            realm_str,
            is_proxy_bool,
        );

        1
    }
}
