// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LoadHandler methods for `AgentMuxHandler` — loading-state, load-end IPC
//! injection + splash signals, and the load-error fallback page. Extracted
//! verbatim from client/mod.rs.

use cef::*;

use super::AgentMuxHandler;

impl AgentMuxHandler {
    /// CEF fires this whenever the browser's loading/history state changes
    /// (navigation started, navigation committed, back/forward enabled).
    /// `can_go_back` / `can_go_forward` come directly from the navigation
    /// controller — no need to query `browser.can_go_back()` (which races
    /// with history commit when called from `on_load_end`).
    ///
    /// For panes: emit `browser-pane-nav-state` so the frontend address
    /// bar + back/forward buttons reflect CEF's real history state.
    pub(crate) fn on_loading_state_change(
        &mut self,
        browser: Option<&mut Browser>,
        _is_loading: i32,
        can_go_back: i32,
        can_go_forward: i32,
    ) {
        if !self.is_browser_pane {
            return;
        }
        if let Some(b) = browser.as_deref() {
            crate::browser_pane::callbacks::on_loading_state_change_browser_pane(
                &self.state,
                b,
                can_go_back != 0,
                can_go_forward != 0,
            );
        }
    }

    pub(crate) fn on_load_end(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        _http_status_code: i32,
    ) {
        // Inject the IPC port into the page after it finishes loading.
        // Only inject into the main frame (not iframes).
        let Some(frame) = frame else { return };

        if frame.is_main() != 1 {
            return;
        }

        // Pane-specific on_load_end work (focus subclass re-install after
        // Chromium rebuilds Chrome_RenderWidgetHostHWND on navigation)
        // lives in `crate::browser_pane::callbacks` after Phase 4. Returning early
        // skips main-only IPC-port injection below.
        if self.is_browser_pane {
            if let Some(b) = browser.as_deref() {
                crate::browser_pane::callbacks::on_load_end_browser_pane(&self.state, b);
            }
            return;
        }

        let ipc_token = &self.state.ipc_token;
        let js = format!(
            "window.__AGENTMUX_IPC_PORT__ = {}; window.__AGENTMUX_IPC_TOKEN__ = '{}';",
            self.ipc_port, ipc_token
        );
        let code = CefString::from(js.as_str());
        let url = CefString::from("");
        frame.execute_java_script(Some(&code), Some(&url), 0);

        let url_str = browser
            .as_ref()
            .and_then(|b| b.main_frame().map(|f| CefString::from(&f.url()).to_string()))
            .unwrap_or_default();
        tracing::info!(
            "Injected IPC port {} into page: {}",
            self.ipc_port,
            url_str
        );

        // Signal the pre-splash to fade out the moment CEF's first frame
        // is ready. The launcher created this named event and forwarded
        // its name via AGENTMUX_SPLASH_EVENT. OpenEventW + SetEvent is
        // fire-and-forget; missing env var means no splash was running.
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenEventW, SetEvent, EVENT_MODIFY_STATE,
            };
            if let Ok(event_name) = std::env::var("AGENTMUX_SPLASH_EVENT") {
                let nul: Vec<u16> = format!("{}\0", event_name).encode_utf16().collect();
                unsafe {
                    let ev = OpenEventW(EVENT_MODIFY_STATE, 0, nul.as_ptr());
                    if !ev.is_null() {
                        SetEvent(ev);
                        CloseHandle(ev);
                    }
                }
            }
        }

        // macOS/Linux analogue of the Win32 splash signal: the launcher owns the
        // native splash (see agentmux-launcher/src/splash_mac.rs and splash_linux/)
        // and passes a ready-file path via AGENTMUX_SPLASH_READY_FILE. Creating the
        // file is the cross-process "first frame painted" signal the launcher polls
        // for before tearing the splash down. Fire-and-forget; absent var => no
        // launcher splash (e.g. dev:standalone), so this is a no-op.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if let Ok(path) = std::env::var("AGENTMUX_SPLASH_READY_FILE") {
                if !path.is_empty() {
                    let _ = std::fs::write(&path, b"ready");
                }
            }
        }

        // Show window via CEF Views API after content paints.
        // All windows (main + secondary) now use CEF Views.
        let mut browser_cloned = browser.cloned();

        // Pool windows are kept hidden until promote_pool_window fires
        // PromotePoolWindowTask, which positions the window with set_bounds()
        // and then calls window.show(). On macOS/Linux, CEF Views Window::Show()
        // activates the widget (no foreground-lock equivalent), so showing an
        // off-screen pool window here — even at (-32000,-32000) — would steal
        // key focus. Instead, pool windows skip the show/focus block entirely
        // and are shown for the first time at the promote-target position.
        let browser_label = browser_cloned.as_mut().and_then(|b| self.window_label_for(b));
        let is_pool_window = browser_label
            .as_deref()
            .map_or(false, |l| l.starts_with("window-pool-") || l.starts_with("floating-pool-"));

        if !is_pool_window {
            if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
                if let Some(window) = bv.window() {
                    if window.is_visible() == 0 {
                        window.show();
                        if let Some(ref mut b) = browser_cloned {
                            if let Some(host) = b.host() {
                                host.set_focus(1);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn on_load_error(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let error_code_raw = sys::cef_errorcode_t::from(error_code);

        // [DIAG] Unconditional entry log — captures every load error
        // BEFORE the ERR_ABORTED filter, sub-frame filter, or fallback-
        // page render. Pair with `[browser-pane-auth][ENTRY]` to
        // diagnose the auth-modal-doesn't-appear path: if we see a
        // load-error here with no preceding auth-credentials entry,
        // CEF skipped the auth flow entirely.
        let failed_url_dbg = failed_url.map(CefString::to_string).unwrap_or_default();
        let error_text_dbg = error_text.map(CefString::to_string).unwrap_or_default();
        // `as_ref()` + auto-deref on `&&mut Frame` — relies only on
        // `is_main(&self)` resolving through normal method-call deref,
        // not on `Deref` for the `Option::as_deref()` blanket impl.
        let is_main_frame = match frame.as_ref() {
            Some(f) => f.is_main() == 1,
            None => false,
        };
        tracing::info!(
            "[load-error][ENTRY] url={:?} error={:?} ({}) is_main_frame={} aborted={}",
            failed_url_dbg,
            error_text_dbg,
            error_code_raw as i32,
            is_main_frame,
            error_code_raw == sys::cef_errorcode_t::ERR_ABORTED,
        );

        // Persistent pane lifecycle trace (see browser_pane::trace). Recorded
        // BEFORE the ERR_ABORTED early-return below, because ERR_ABORTED on a
        // pane's main frame is exactly the black-render-on-redock signature
        // (the re-created pane's navigation was aborted mid-load).
        if self.is_browser_pane && is_main_frame {
            if let Some(b) = browser.as_deref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    crate::browser_pane::trace::pane_trace(
                        &block_id,
                        "load-error",
                        &format!(
                            "url={failed_url_dbg} err={error_text_dbg}({}) aborted={}",
                            error_code_raw as i32,
                            error_code_raw == sys::cef_errorcode_t::ERR_ABORTED,
                        ),
                    );
                }
            }
        }

        if error_code_raw == sys::cef_errorcode_t::ERR_ABORTED {
            return;
        }

        let frame = frame.expect("Frame is None");

        // Don't show error pages for sub-frames (iframes) — only for
        // the main frame. Without this, an iframe blocked by
        // X-Frame-Options replaces the entire app with an error page.
        if frame.is_main() != 1 {
            return;
        }
        let error_text = error_text.map(CefString::to_string).unwrap_or_default();
        let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
        let error_code_i32 = error_code_raw as i32;

        // JSON-encode the URL so it is a safe JS string literal: a real URL can
        // contain a single quote (e.g. `?q=can't`), which would otherwise break
        // the interpolated JS in the Retry handler below.
        let failed_url_js =
            serde_json::to_string(&failed_url).unwrap_or_else(|_| "\"\"".to_string());
        // Auto-retry ONLY for the dev frontend (the main window), which commonly
        // races the Vite dev server on launch. Browser panes load arbitrary user
        // URLs through this SAME handler — auto-retrying their failures (offline
        // site, DNS error, refused service) would be an unbounded reload loop, so
        // panes get a manual Retry only.
        let auto_retry = if self.is_browser_pane {
            String::new()
        } else {
            "setTimeout(__amxRetry, 1200);".to_string()
        };

        tracing::error!(
            "Load error: url={} error={} ({})",
            failed_url,
            error_text,
            error_code_i32
        );

        // Show a user-friendly error page.
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1e1e2e;
            color: #cdd6f4;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
        }}
        .error-container {{
            text-align: center;
            max-width: 600px;
            padding: 40px;
        }}
        h1 {{ color: #f38ba8; font-size: 24px; }}
        p {{ color: #a6adc8; line-height: 1.6; }}
        code {{
            background: #313244;
            padding: 2px 8px;
            border-radius: 4px;
            font-size: 14px;
        }}
        .retry {{
            margin-top: 20px;
            padding: 10px 24px;
            background: #89b4fa;
            color: #1e1e2e;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            font-size: 14px;
        }}
    </style>
</head>
<body>
    <div class="error-container">
        <h1>Failed to load AgentMux frontend</h1>
        <p>Could not connect to <code>{failed_url}</code></p>
        <p>Error: {error_text} ({error_code_i32})</p>
        <p>Make sure the Vite dev server is running:<br>
           <code>task dev</code> or <code>npx vite</code></p>
        <button class="retry" onclick="__amxRetry()">Retry</button>
    <script>
        // This error page is itself a data: URI, so location.reload() would just
        // reload THIS page (and a data: reload aborts) — the original URL would
        // never be re-tried. Navigate to the real failed URL instead.
        var __amxTarget = {failed_url_js};
        function __amxRetry() {{ location.href = __amxTarget; }}
        {auto_retry}
    </script>
    </div>
</body>
</html>"#
        );

        let b64 = cef::base64_encode(Some(html.as_bytes()));
        let b64_str = CefString::from(&b64).to_string();
        let data_uri = format!("data:text/html;base64,{}", b64_str);
        let uri = CefString::from(data_uri.as_str());
        frame.load_url(Some(&uri));
    }
}
