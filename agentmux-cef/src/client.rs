// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefClient and associated handler implementations.
// Manages browser lifecycle, display updates, and load errors.
//
// Phase 2: Stores browser ref in AppState and injects IPC port on page load.

use cef::*;
use std::sync::Arc;
use parking_lot::Mutex;

use crate::state::{AppState, WindowKind};

/// Write a debug line to `%TEMP%\agentmux-close-debug.txt`.
///
/// Only active when `AGENTMUX_DEBUG_CLOSE=1` is set in the environment.
/// In normal production runs the file is never written to.
/// Always emits at tracing::debug level regardless of the env flag.
pub fn dlog(msg: &str) {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| std::env::var("AGENTMUX_DEBUG_CLOSE").is_ok());

    tracing::debug!("[close-debug] {}", msg);

    if enabled {
        use std::io::Write;
        let path = std::env::temp_dir().join("agentmux-close-debug.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
            let _ = writeln!(f, "[{}] {}", ms, msg);
        }
        tracing::info!("[close-debug] {}", msg);
    }
}

/// Count of pane-close operations currently in flight between
/// `BrowserPaneManager::close()` and the pane's `on_before_close`. When > 0,
/// main's `do_close` cancels — empirically, tearing down a pane Alloy browser
/// (child HWND of main's top-level) cascades a WM_CLOSE-equivalent into main,
/// emptying main's browser_list and firing `quit_message_loop`, which exits
/// the whole app. See docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md §4 and the
/// host-log trace at v0.33.251 08:56:19.777 ("Unregistered browser:
/// label=main") for evidence.
pub static PANE_CLOSE_IN_PROGRESS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Core handler state shared across all CEF callback interfaces.
pub struct AgentMuxHandler {
    browser_list: Vec<Browser>,
    is_closing: bool,
    state: Arc<AppState>,
    ipc_port: u16,
    is_pane: bool,
}

impl AgentMuxHandler {
    pub fn new(state: Arc<AppState>, ipc_port: u16) -> Arc<Mutex<Self>> {
        Self::new_with_pane(state, ipc_port, false)
    }

    pub fn new_with_pane(state: Arc<AppState>, ipc_port: u16, is_pane: bool) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            browser_list: Vec::new(),
            is_closing: false,
            state,
            ipc_port,
            is_pane,
        }))
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Update the window title via CEF Views.
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        }
        // For Alloy-style native windows on Windows, update via Win32 API.
        #[cfg(target_os = "windows")]
        {
            if let (Some(browser), Some(title)) = (browser.as_ref(), title) {
                if let Some(host) = browser.host() {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        let title_wide: Vec<u16> = title
                            .to_string()
                            .encode_utf16()
                            .chain(std::iter::once(0))
                            .collect();
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                                hwnd.0 as *mut std::ffi::c_void,
                                title_wide.as_ptr(),
                            );
                        }
                    }
                }
            }
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let browser = browser.cloned().expect("Browser is None");
        tracing::info!("Browser created (total: {})", self.browser_list.len() + 1);

        // Register browser in the multi-window map.
        // First browser is "main"; additional browsers get labels from their URL params.
        let label = {
            let mut browsers = self.state.browsers.lock();
            let label = if browsers.is_empty() {
                "main".to_string()
            } else {
                // Pop the label that was queued by open_new_window / open_window_at_position
                // before posting to the UI thread.  The URL is not yet loaded when
                // on_after_created fires, so reading it here always returns empty string.
                let lbl = self.state.pending_window_labels.lock().pop_front()
                    .unwrap_or_else(|| format!("window-{}", uuid::Uuid::new_v4()));
                dlog(&format!("on_after_created: popped label={}", lbl));
                lbl
            };
            tracing::info!("Registered browser: label={} (total: {})", label, browsers.len() + 1);
            dlog(&format!("on_after_created: registered label={} total={}", label, browsers.len() + 1));
            browsers.insert(label.clone(), browser.clone());
            label
        };

        // Ensure WindowMeta exists for this browser so downstream code (taskbar
        // treatment here, cascade-close below) can classify it. If the creator
        // didn't pre-register a meta (legacy callers, or the very first "main"
        // window), default to FullInstance. Browser panes use `browser-pane-*`
        // labels which are intentionally absent from window_meta — they're
        // child HWNDs, not top-level windows, and skipping the taskbar logic
        // below for them is correct.
        let is_top_level_window = !label.starts_with("browser-pane-");
        if is_top_level_window {
            let mut metas = self.state.window_meta.lock();
            metas.entry(label.clone()).or_insert_with(|| {
                crate::state::WindowMeta {
                    label: label.clone(),
                    kind: WindowKind::FullInstance,
                    parent_instance_id: None,
                }
            });
        }

        // No DwmExtendFrameIntoClientArea — it causes the white flash.
        // CEF Views handles frameless + resize via its delegate.

        // Set the taskbar/title bar icon from the embedded exe resource, and
        // for `Subwindow` top-levels, hide them from the taskbar via
        // ITaskbarList::DeleteTab.
        #[cfg(target_os = "windows")]
        {
            // Prefer CEF Views' `Window::window_handle()` — it targets the
            // specific top-level window for THIS browser, avoiding the
            // `find_own_top_level_window` fallback's "first visible HWND"
            // ambiguity when multiple windows exist.
            let mut browser_mut = browser.clone();
            let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser_mut))
                .and_then(|bv| bv.window())
                .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                .filter(|p| !p.is_null());

            let hwnd = views_top_hwnd.unwrap_or_else(|| {
                browser.host()
                    .and_then(|h| {
                        let wh = h.window_handle();
                        if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                    })
                    .unwrap_or_else(|| unsafe {
                        crate::commands::window::find_own_top_level_window()
                    })
            });

            if !hwnd.is_null() {
                unsafe { set_window_icon(hwnd); }

                // Subwindow? Hide from taskbar. Full instances and browser-pane
                // child HWNDs skip this branch.
                if is_top_level_window {
                    let kind = self.state.window_meta.lock().get(&label).map(|m| m.kind);
                    if kind == Some(WindowKind::Subwindow) {
                        unsafe { skip_taskbar(hwnd); }
                    }
                }
            }
        }

        // Browser panes must sit ABOVE the main browser's widget in Z-order
        // or mouse-wheel events over the visible pane area hit main instead
        // of the pane. Bring the pane's HWND to the top of its parent's
        // Z-order without changing its activation state.
        #[cfg(target_os = "windows")]
        if self.is_pane {
            if let Some(host) = browser.host() {
                let wh = host.window_handle();
                if !wh.0.is_null() {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            wh.0 as _,
                            std::ptr::null_mut(), // HWND_TOP
                            0, 0, 0, 0,
                            0x0001 | 0x0002 | 0x0010, // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
                        );
                    }
                    tracing::info!("[pane-zorder] raised pane to top of Z-order");
                }
            }
        }

        self.browser_list.push(browser);
    }

    fn do_close(&mut self, _browser: Option<&mut Browser>) -> bool {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Cancel the cascade: when a browser pane is mid-close, CEF on Alloy
        // fires do_close on main too (likely because the pane's outer HWND
        // is a child of main's top-level and Chromium's Alloy teardown
        // propagates). Returning true here tells CEF to keep main's browser
        // alive; the pane close still proceeds normally in its own handler.
        // See SPEC_BROWSER_PANE_LIFECYCLE.md §4 trace and the v0.33.251
        // host log where main was torn down 6ms after the pane's
        // on_before_close fired.
        use std::sync::atomic::Ordering;
        if !self.is_pane && PANE_CLOSE_IN_PROGRESS.load(Ordering::SeqCst) > 0 {
            tracing::warn!(
                "[do_close] cancelling main do_close during pane close cascade (count={})",
                PANE_CLOSE_IN_PROGRESS.load(Ordering::SeqCst)
            );
            return true; // cancel
        }

        if self.browser_list.len() == 1 {
            self.is_closing = true;
        }
        // Return false to allow the close.
        false
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        dlog(&format!("on_before_close fired; browser_list.len()={}", self.browser_list.len()));

        let mut browser = browser.cloned().expect("Browser is None");

        // Unregister browser from the multi-window map and get its label.
        let label = {
            let mut browsers = self.state.browsers.lock();
            let keys: Vec<String> = browsers.keys().cloned().collect();
            dlog(&format!("browsers map keys: {:?}", keys));
            let label = browsers.iter()
                .find(|(_, b)| b.is_same(Some(&mut browser)) != 0)
                .map(|(k, _)| k.clone());
            dlog(&format!("label found: {:?}", label));
            if let Some(ref lbl) = label {
                browsers.remove(lbl);
                tracing::info!("Unregistered browser: label={} (remaining: {})", lbl, browsers.len());
            }
            label
        };

        // Drain the pane's lifecycle entry so a re-create with the same block_id
        // gets a fresh Live state. Label prefix tells us this was a pane browser;
        // it's cheap enough to always call — the manager's drain is a no-op for
        // labels it doesn't own.
        if let Some(ref lbl) = label {
            if lbl.starts_with("browser-pane-") {
                self.state.browser_panes.drain_closed_label(lbl);
            }
        }

        // Unregister from instance registry; retrieve backend window ID for cleanup.
        let backend_window_id = label.as_deref().and_then(|lbl| {
            self.state.window_instance_registry.lock().unregister(lbl);
            let wid = self.state.window_id_map.lock().remove(lbl);
            dlog(&format!("window_id_map.remove({:?}) => {:?}", lbl, wid));
            wid
        });

        // Pull and remove the closing window's meta; if it's a FullInstance,
        // cascade-close every Subwindow whose parent_instance_id points to it.
        // See `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md` §2.3.
        let closing_meta = label
            .as_deref()
            .and_then(|lbl| self.state.window_meta.lock().remove(lbl));
        if let Some(meta) = &closing_meta {
            if meta.kind == WindowKind::FullInstance {
                let child_labels: Vec<String> = self
                    .state
                    .window_meta
                    .lock()
                    .values()
                    .filter(|m| {
                        m.kind == WindowKind::Subwindow
                            && m.parent_instance_id.as_deref() == Some(&meta.label)
                    })
                    .map(|m| m.label.clone())
                    .collect();
                for child_label in child_labels {
                    if let Some(mut child) = self.state.browsers.lock().get(&child_label).cloned() {
                        if let Some(host) = child.host() {
                            tracing::info!(parent = %meta.label, child = %child_label, "[subwindow-cascade] closing sub-window");
                            host.close_browser(1);
                        }
                    }
                }
            }
        }

        dlog(&format!("backend_window_id: {:?}", backend_window_id));

        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }

        dlog(&format!("browser_list after remove: {}", self.browser_list.len()));

        if self.browser_list.is_empty() && !self.is_pane {
            dlog("last window — calling quit_message_loop");
            // Last window — quit the message loop.
            // The Job Object kills agentmux-srv which kills all shell processes.
            // Only the MAIN client's handler should trigger app exit — pane
            // clients own only their own pane browser and their list emptying
            // just means the user closed the pane, not the whole app.
            quit_message_loop();
        } else {
            // Notify remaining windows of the new count.
            let new_count = self.state.window_instance_registry.lock().count();
            crate::events::emit_event_all_windows(
                &self.state,
                "window-instances-changed",
                &serde_json::json!(new_count),
            );

            // Tell the backend to clean up this window's workspace/tabs/shells.
            // This replaces the JavaScript `beforeunload` handler — running it here
            // ensures shells die after the CEF browser is gone (not while it's still
            // alive), so Task Manager keeps them grouped until they exit.
            if let Some(window_id) = backend_window_id {
                let web_endpoint = self.state.backend_endpoints.lock().web_endpoint.clone();
                let auth_key = self.state.auth_key.lock().clone();
                dlog(&format!("spawning backend_close_window thread for window_id={}", window_id));
                std::thread::spawn(move || {
                    backend_close_window(&web_endpoint, &auth_key, &window_id);
                });
            } else {
                let warn = format!(
                    "[on_before_close] no backend window ID registered for label={:?} — shells may orphan",
                    label
                );
                dlog(&warn);
                tracing::warn!("{}", warn);
            }
        }
    }

    fn on_load_end(
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

        // For browser panes: skip IPC-port injection (the embedded page is
        // not our frontend). Re-install the HWND subclass — Chromium creates
        // the render HWND (Chrome_RenderWidgetHostHWND) during navigation, so
        // it doesn't exist yet in on_after_created. We re-subclass on every
        // navigation so newly-created child HWNDs are always covered.
        //
        // DO NOT force focus back to main here: WM_MOUSEWHEEL is routed to
        // the focused HWND (on Windows without "scroll inactive windows"),
        // so stealing focus away from the pane breaks scrolling. The
        // FocusHandler cancel + WndProc redirect already keep focus off the
        // pane during the *initial* navigation focus steal; user-driven
        // focus goes through `browser_pane_focus` IPC, and clicking a main
        // input goes through `main_window_focus`.
        if self.is_pane {
            tracing::info!("[pane-load-end] pane page loaded");
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

        // Show window via CEF Views API after content paints.
        // All windows (main + secondary) now use CEF Views.
        let mut browser_cloned = browser.cloned();
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

    fn on_load_error(
        &mut self,
        _browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let error_code_raw = sys::cef_errorcode_t::from(error_code);
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
        <button class="retry" onclick="location.reload()">Retry</button>
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

    /// Render-process terminated — typically OOM, a renderer-side panic, or
    /// some native bug inside CEF/Chromium. Without this hook the window
    /// just turns white. We log the cause and replace the white page with
    /// a recovery HTML page that offers Reload / Quit buttons.
    ///
    /// See specs/SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).
    fn on_render_process_terminated(
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
        tracing::error!(
            target: "crash",
            kind = "renderer_terminated",
            reason,
            error_code,
            detail = %detail,
            "{}", reason,
        );

        // Resolve the real frontend URL so the Reload button can navigate
        // back to the live app instead of reloading the recovery page
        // itself. Matches the format used by
        // commands::window::resolve_frontend_base_url and its callers
        // (see window.rs:400, window.rs:430, drag.rs:294 — all use the
        // same ipc_port / ipc_token query params).
        let base_url = crate::commands::window::resolve_frontend_base_url(self.ipc_port);
        let separator = if base_url.contains('?') { "&" } else { "?" };
        let app_url = format!(
            "{}{}ipc_port={}&ipc_token={}",
            base_url, separator, self.ipc_port, self.state.ipc_token
        );

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
}

/// Quote a string as a JavaScript string literal — escape backslashes,
/// quotes, and newlines so it's safe to embed inside `<script>` via
/// `format!`. Used by the recovery page to inject the app URL for the
/// Reload button's navigation target.
fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"), // defense against </script> injection
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Minimal HTML escape for the recovery page. Only the characters that
/// would break the `format!`-templated string need attention; the input
/// (CEF status enum + cef-provided error string) is trusted but may
/// contain `&` / `<` / `>` in some failure modes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// CefClient — routes to sub-handlers
// ---------------------------------------------------------------------------

wrap_client! {
    pub struct AgentMuxClient {
        inner: Arc<Mutex<AgentMuxHandler>>,
        is_pane: bool,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(AgentMuxDisplayHandler::new(self.inner.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(AgentMuxKeyboardHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(AgentMuxLifeSpanHandler::new(self.inner.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(AgentMuxLoadHandler::new(self.inner.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(AgentMuxRequestHandler::new(self.inner.clone()))
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            // For browser panes only: cancel CEF's auto-focus on navigation so the
            // child HWND doesn't steal keyboard focus from the main window when the
            // page finishes loading. The user can still click into the pane to focus it.
            if self.is_pane {
                Some(AgentMuxPaneFocusHandler::new())
            } else {
                None
            }
        }
    }
}

// FocusHandler used only by browser-pane clients — cancels NAVIGATION focus
// (triggered by page load) but allows SYSTEM focus (user click, keyboard).
// Combined with the Win32 WndProc subclass below, this means:
//   • Page load / JS window.focus() → cancelled at both Chromium and Win32
//   • User click inside the pane → allowed at both layers, pane gets focus,
//     user can type into page inputs (e.g. google.com search box)
wrap_focus_handler! {
    struct AgentMuxPaneFocusHandler;

    impl FocusHandler {
        fn on_set_focus(
            &self,
            _browser: Option<&mut Browser>,
            source: FocusSource,
        ) -> ::std::os::raw::c_int {
            let cancel = source == FocusSource::NAVIGATION;
            tracing::info!("[pane-focus] on_set_focus source={:?} cancel={}", source, cancel);
            if cancel { 1 } else { 0 }
        }
    }
}

// ---------------------------------------------------------------------------
// KeyboardHandler — intercept Ctrl+<key> shortcuts before CEF/Chromium
// consumes them (e.g., Ctrl+P = print, Ctrl+G = find-next).
// Returning true from on_pre_key_event tells CEF "handled" so it won't
// trigger the built-in action; the key still reaches JavaScript.
// ---------------------------------------------------------------------------

/// CEF event flag: Ctrl key is held.
const EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;

/// Windows virtual-key codes for shortcuts we want to forward to JS.
const VK_P: i32 = 0x50; // Ctrl+P — command palette (not print)
const VK_G: i32 = 0x47; // Ctrl+G — (reserve for app use)

wrap_keyboard_handler! {
    struct AgentMuxKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut cef::sys::MSG>,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let Some(ev) = event {
                let ctrl = (ev.modifiers & EVENTFLAG_CONTROL_DOWN) != 0;
                if ctrl && matches!(ev.windows_key_code, VK_P | VK_G) {
                    // Tell CEF this is a keyboard shortcut so it dispatches
                    // the keydown event to JavaScript instead of handling it
                    // as a built-in browser action (print dialog, etc.).
                    if let Some(flag) = is_keyboard_shortcut {
                        *flag = 1;
                    }
                    // Return 0 = not consumed at pre-key stage; CEF will
                    // still call on_key_event where we return 0 again,
                    // letting JS handle it via the normal keydown path.
                }
            }
            0 // not consumed
        }
    }
}

// ---------------------------------------------------------------------------
// DisplayHandler — title changes
// ---------------------------------------------------------------------------

wrap_display_handler! {
    struct AgentMuxDisplayHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let mut inner = self.inner.lock();
            inner.on_title_change(browser, title);
        }
    }
}

// ---------------------------------------------------------------------------
// LifeSpanHandler — browser creation/destruction
// ---------------------------------------------------------------------------

wrap_life_span_handler! {
    struct AgentMuxLifeSpanHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock();
            inner.on_after_created(browser);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let mut inner = self.inner.lock();
            inner.do_close(browser).into()
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock();
            inner.on_before_close(browser);
        }
    }
}

// ---------------------------------------------------------------------------
// LoadHandler — load events and errors
// ---------------------------------------------------------------------------

wrap_load_handler! {
    struct AgentMuxLoadHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            let mut inner = self.inner.lock();
            inner.on_load_end(browser, frame, http_status_code);
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock();
            inner.on_load_error(browser, frame, error_code, error_text, failed_url);
        }
    }
}

// ---------------------------------------------------------------------------
// RequestHandler — render-process termination (white-screen recovery)
// ---------------------------------------------------------------------------
//
// We only override `on_render_process_terminated` here. Everything else
// inherits the default (no-op) implementations from the cef-rs trait.
// See SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).

wrap_request_handler! {
    struct AgentMuxRequestHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl RequestHandler {
        fn on_render_process_terminated(
            &self,
            browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: ::std::os::raw::c_int,
            error_string: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock();
            inner.on_render_process_terminated(browser, status, error_code, error_string);
        }
    }
}

/// Set up a native frameless window: extend client area over the thick frame
/// border so the resize handle is invisible, then subclass the window to
/// handle WM_NCHITTEST for edge resize.
///
/// DwmExtendFrameIntoClientArea(-1) makes the entire frame transparent, but
/// it also removes the non-client hit-test region. Without the subclass,
/// Windows can't tell which part of the window edge should be a resize handle.
/// The subclass returns HT{LEFT,RIGHT,TOP,BOTTOM,TOPLEFT,...} when the cursor
/// is within RESIZE_BORDER pixels of the window edge.
#[cfg(target_os = "windows")]
unsafe fn setup_native_frameless(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows_sys::Win32::UI::Controls::MARGINS;

    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let result = DwmExtendFrameIntoClientArea(hwnd, &margins);
    if result == 0 {
        tracing::info!("Applied DwmExtendFrameIntoClientArea to hide resize border");
    } else {
        tracing::warn!("DwmExtendFrameIntoClientArea failed: hr={:#x}", result);
    }
}

/// Map of HWND -> original WndProc for secondary windows with edge resize hooks.
/// Stored here instead of GWLP_USERDATA to avoid clobbering CEF's data.
#[cfg(target_os = "windows")]
static ORIGINAL_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Map of pane HWND -> original WndProc for the focus-redirect subclass below.
#[cfg(target_os = "windows")]
static PANE_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Subclass a browser pane's HWND so that WM_SETFOCUS is immediately redirected
/// back to the parent (top-level) window. Without this, Chromium's internal
/// SetFocus on the pane HWND (on page load, on JS window.focus() calls, etc.)
/// steals the Windows-level keyboard focus — any subsequent keystrokes go to
/// the pane's renderer instead of the main window, so terminals, URL bars, and
/// other inputs in the main UI stop responding.
///
/// The pane becomes view-only (user cannot type into google.com's search box),
/// but the main window keeps working. A later gesture (e.g. Alt+click) can
/// restore focus to the pane explicitly when the user wants to interact with
/// the embedded page.
/// When true, the next WM_SETFOCUS delivered to a subclassed pane HWND is
/// allowed through instead of being redirected. The `browser_pane_focus` IPC
/// handler sets this flag before calling SetFocus on the pane, so user-
/// initiated focus (routed by the frontend's ViewModel.giveFocus()) works
/// even though the Chromium-internal focus steal on navigation is blocked.
#[cfg(target_os = "windows")]
pub static ALLOW_PANE_FOCUS_ONCE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "windows")]
unsafe fn install_pane_focus_redirect(hwnd: *mut std::ffi::c_void) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GetParent, SetWindowLongPtrW, GWLP_WNDPROC, WM_SETFOCUS,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        // Diagnostic: surface mouse-wheel and key events so we can tell
        // whether they reach the pane HWND at all when the user reports
        // scrolling/typing breakage.
        const WM_MOUSEWHEEL: u32 = 0x020A;
        const WM_MOUSEHWHEEL: u32 = 0x020E;
        const WM_KEYDOWN: u32 = 0x0100;
        const WM_CHAR: u32 = 0x0102;
        match msg {
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                tracing::info!("[pane-wndproc] mouse-wheel hwnd={:p} msg=0x{:x}", hwnd, msg);
            }
            WM_KEYDOWN | WM_CHAR => {
                tracing::info!("[pane-wndproc] key msg=0x{:x} wparam={}", msg, wparam);
            }
            _ => {}
        }

        if msg == WM_SETFOCUS {
            // Intentional focus from the frontend's giveFocus() IPC: honor it
            // once, then revert to redirect-mode for subsequent events.
            if ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed) {
                tracing::info!("[pane-wndproc] WM_SETFOCUS allowed (intentional)");
                // Fall through to the original WndProc.
            } else {
                // Programmatic focus (page load, JS window.focus()): redirect.
                let parent = GetParent(hwnd);
                if !parent.is_null() {
                    SetFocus(parent);
                }
                return 0;
            }
        }

        let original = PANE_WNDPROCS
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        if original != 0 {
            let proc_fn: unsafe extern "system" fn(
                *mut std::ffi::c_void, u32, usize, isize,
            ) -> isize = std::mem::transmute(original);
            CallWindowProcW(Some(proc_fn), hwnd, msg, wparam, lparam)
        } else {
            0
        }
    }

    // Subclass the outer HWND — but only once. Re-calling SetWindowLongPtrW
    // would replace our hook with itself and poison PANE_WNDPROCS.
    let already_hooked = PANE_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if !already_hooked {
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if original != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(hwnd as usize, original);
            }
            tracing::info!("[pane-subclass] installed focus-redirect WndProc on pane HWND {:p}", hwnd);
        }
    }

    // Chromium creates inner HWNDs (widget + render) below the outer HWND.
    // Mouse input reaches the deepest descendant, so we must walk the whole
    // tree and subclass every one.
    unsafe extern "system" fn enum_children(
        child: *mut std::ffi::c_void,
        _lparam: isize,
    ) -> i32 {
        let already = PANE_WNDPROCS
            .lock()
            .ok()
            .map(|m| m.contains_key(&(child as usize)))
            .unwrap_or(false);
        if already {
            return 1;
        }
        let orig = SetWindowLongPtrW(child, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if orig != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(child as usize, orig);
            }
            let mut class_buf = [0u16; 64];
            let n = windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                child, class_buf.as_mut_ptr(), class_buf.len() as i32,
            );
            let class_name = String::from_utf16_lossy(&class_buf[..n as usize]);
            tracing::info!("[pane-subclass] subclassed child HWND {:p} class={}", child, class_name);
        }
        1 // continue
    }
    windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        hwnd, Some(enum_children), 0,
    );
}

/// Install a WndProc hook on a SECONDARY window that handles:
/// - WM_NCCALCSIZE: returns 0 to eliminate the non-client area (removes the
///   wide title bar / top border that WS_THICKFRAME + DWM extension creates)
/// - WM_NCHITTEST: returns HT{LEFT,RIGHT,...} for resize zones at window edges
///
/// MUST NOT be installed on the main CEF Views window — that window handles
/// resize through its delegate, and hooking it clobbers CEF internals.
#[cfg(target_os = "windows")]
unsafe fn install_frameless_resize_hook(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const RESIZE_BORDER: i32 = 6;

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        match msg {
            // Remove the non-client area entirely — this eliminates the wide
            // top border that WS_THICKFRAME normally reserves for the title bar.
            WM_NCCALCSIZE if wparam == 1 => {
                // Returning 0 with wparam=1 tells Windows the client area
                // fills the entire window rect. No title bar, no borders.
                return 0;
            }

            // Suppress the DWM activation border — return TRUE without
            // calling DefWindowProc so Windows doesn't repaint the frame.
            WM_NCACTIVATE => {
                return 1; // TRUE = allow activation, but skip default border paint
            }

            WM_NCHITTEST => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

                let mut rect = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                GetWindowRect(hwnd, &mut rect);

                let left = x - rect.left < RESIZE_BORDER;
                let right = rect.right - x < RESIZE_BORDER;
                let top = y - rect.top < RESIZE_BORDER;
                let bottom = rect.bottom - y < RESIZE_BORDER;

                if top && left { return HTTOPLEFT as isize; }
                if top && right { return HTTOPRIGHT as isize; }
                if bottom && left { return HTBOTTOMLEFT as isize; }
                if bottom && right { return HTBOTTOMRIGHT as isize; }
                if left { return HTLEFT as isize; }
                if right { return HTRIGHT as isize; }
                if top { return HTTOP as isize; }
                if bottom { return HTBOTTOM as isize; }
                // Not on an edge — fall through to original WndProc.
            }

            _ => {}
        }

        // Delegate to the original WndProc.
        let key = hwnd as usize;
        let original = ORIGINAL_WNDPROCS
            .lock()
            .ok()
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(0);
        if original != 0 {
            CallWindowProcW(Some(std::mem::transmute(original)), hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    let original = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
    if let Ok(mut map) = ORIGINAL_WNDPROCS.lock() {
        map.insert(hwnd as usize, original);
    }
    SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as isize);
    tracing::info!("Installed frameless resize hook (WM_NCCALCSIZE + WM_NCHITTEST)");
}

/// Hide the given top-level HWND from the Windows taskbar via
/// `ITaskbarList::DeleteTab`. The window remains fully usable — Alt-Tab still
/// finds it, it takes focus, repaints, etc. — but the shell paints no taskbar
/// button for it regardless of the user's "Combine taskbar buttons" setting.
///
/// Used only for `WindowKind::Subwindow` top-level windows. Must be called
/// once the HWND exists (post-`on_after_created`) and re-applied on the
/// `TaskbarCreated` broadcast after Explorer restarts.
///
/// Same primitive Electron uses in `NativeWindowViews::SetSkipTaskbar`
/// (`shell/browser/native_window_views.cc`).
#[cfg(target_os = "windows")]
unsafe fn skip_taskbar(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows_sys::core::GUID;

    // CLSID_TaskbarList
    const CLSID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF344,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };
    // IID_ITaskbarList
    const IID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF342,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };

    // Hand-rolled vtable — `windows-sys` doesn't expose `ITaskbarList` types
    // at this feature level, and pulling in the full `windows` crate for one
    // COM interface is overkill.
    #[repr(C)]
    struct ITaskbarList {
        lp_vtbl: *const ITaskbarListVtbl,
    }
    #[repr(C)]
    struct ITaskbarListVtbl {
        query_interface: unsafe extern "system" fn(*mut ITaskbarList, *const GUID, *mut *mut core::ffi::c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut ITaskbarList) -> u32,
        release: unsafe extern "system" fn(*mut ITaskbarList) -> u32,
        hr_init: unsafe extern "system" fn(*mut ITaskbarList) -> i32,
        add_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        delete_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        activate_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        set_active_alt: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
    }

    let mut tbl: *mut ITaskbarList = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &CLSID_TASKBAR_LIST as *const GUID,
        std::ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_TASKBAR_LIST as *const GUID,
        &mut tbl as *mut _ as *mut _,
    );
    if hr < 0 || tbl.is_null() {
        tracing::warn!("[skip_taskbar] CoCreateInstance(TaskbarList) failed: hr=0x{:x}", hr);
        return;
    }

    let vtbl = &*(*tbl).lp_vtbl;
    let hr = (vtbl.hr_init)(tbl);
    if hr < 0 {
        tracing::warn!("[skip_taskbar] HrInit failed: hr=0x{:x}", hr);
        (vtbl.release)(tbl);
        return;
    }
    let hr = (vtbl.delete_tab)(tbl, hwnd);
    if hr < 0 {
        tracing::warn!("[skip_taskbar] DeleteTab failed: hr=0x{:x}", hr);
    } else {
        tracing::info!("[skip_taskbar] hid HWND {:p} from taskbar", hwnd);
    }
    (vtbl.release)(tbl);
}

/// Load the app icon from the exe's embedded resource and set it on the window.
/// This makes the icon appear in the taskbar and Alt+Tab switcher instead of
/// the default CEF/Chromium icon.
#[cfg(target_os = "windows")]
unsafe fn set_window_icon(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    let hinstance = GetModuleHandleW(std::ptr::null());
    if hinstance.is_null() {
        tracing::warn!("set_window_icon: GetModuleHandleW returned null");
        return;
    }

    // Load the big icon (32x32, for Alt+Tab / taskbar)
    let icon_big = LoadImageW(
        hinstance,
        1 as *const u16, // Resource ID 1 (set by winres)
        IMAGE_ICON,
        32, 32,
        LR_SHARED,
    );
    if !icon_big.is_null() {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon_big as isize);
    }

    // Load the small icon (16x16, for title bar)
    let icon_small = LoadImageW(
        hinstance,
        1 as *const u16,
        IMAGE_ICON,
        16, 16,
        LR_SHARED,
    );
    if !icon_small.is_null() {
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon_small as isize);
    }

    if !icon_big.is_null() || !icon_small.is_null() {
        tracing::info!("Set window icon from embedded resource");
    } else {
        tracing::warn!("set_window_icon: no icon found in exe resource");
    }
}

/// Synchronously tell the backend to close a window's workspace/tabs/shells.
///
/// Uses a raw TCP connection so no async runtime or extra crate is needed.
/// Called from a background thread in `on_before_close` so the CEF UI thread
/// is not blocked. Fire-and-forget: we write the request and don't read the response.
fn backend_close_window(web_endpoint: &str, auth_key: &str, window_id: &str) {
    use std::io::Write;

    // Parse host:port from "http://127.0.0.1:PORT"
    let addr_str = web_endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let addr: std::net::SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[backend_close_window] cannot parse endpoint '{}': {}", web_endpoint, e);
            return;
        }
    };

    let body = serde_json::json!({
        "service": "window",
        "method": "CloseWindow",
        "args": [window_id],
        "uicontext": null,
    }).to_string();
    let request = format!(
        "POST /wave/service?service=window&method=CloseWindow&authkey={} HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key, body.len(), body
    );

    dlog(&format!("backend_close_window: connecting to {} for window_id={}", addr, window_id));
    let timeout = std::time::Duration::from_millis(2000);
    match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            stream.set_write_timeout(Some(timeout)).ok();
            stream.set_read_timeout(Some(timeout)).ok();
            match stream.write_all(request.as_bytes()) {
                Ok(_) => {
                    dlog(&format!("backend_close_window: sent request for window_id={}", window_id));
                    // Read response to confirm the backend received it
                    use std::io::Read;
                    let mut resp = String::new();
                    let _ = stream.read_to_string(&mut resp);
                    let first_line = resp.lines().next().unwrap_or("(empty)").to_string();
                    dlog(&format!("backend_close_window: response first line: {}", first_line));
                }
                Err(e) => dlog(&format!("backend_close_window: write failed: {}", e)),
            }
        }
        Err(e) => dlog(&format!("backend_close_window: connect failed to {}: {}", addr, e)),
    }
}
