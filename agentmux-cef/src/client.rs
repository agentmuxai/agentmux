// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefClient and associated handler implementations.
// Manages browser lifecycle, display updates, and load errors.
//
// Phase 2: Stores browser ref in AppState and injects IPC port on page load.

use cef::*;
use std::sync::{Arc, Mutex};

use crate::state::AppState;

/// Core handler state shared across all CEF callback interfaces.
pub struct AgentMuxHandler {
    browser_list: Vec<Browser>,
    is_closing: bool,
    state: Arc<AppState>,
    ipc_port: u16,
}

impl AgentMuxHandler {
    pub fn new(state: Arc<AppState>, ipc_port: u16) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            browser_list: Vec::new(),
            is_closing: false,
            state,
            ipc_port,
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

        // Store the browser handle in AppState for IPC event emission.
        // Only store the first (main) browser.
        {
            let mut state_browser = self.state.browser.lock().unwrap();
            if state_browser.is_none() {
                *state_browser = Some(browser.clone());
                tracing::info!("Stored main browser handle in AppState");
            }
        }

        self.browser_list.push(browser);
    }

    fn do_close(&mut self, _browser: Option<&mut Browser>) -> bool {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        if self.browser_list.len() == 1 {
            self.is_closing = true;
        }
        // Return false to allow the close.
        false
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let mut browser = browser.cloned().expect("Browser is None");

        // Clear the browser handle from AppState if this is the main browser.
        {
            let mut state_browser = self.state.browser.lock().unwrap();
            if let Some(ref stored) = *state_browser {
                if stored.is_same(Some(&mut browser)) != 0 {
                    *state_browser = None;
                    tracing::info!("Cleared main browser handle from AppState");
                }
            }
        }

        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }

        tracing::info!(
            "Browser closed (remaining: {})",
            self.browser_list.len()
        );

        if self.browser_list.is_empty() {
            // All browsers closed — quit the message loop.
            quit_message_loop();
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

        let js = format!(
            "window.__AGENTMUX_IPC_PORT__ = {};",
            self.ipc_port
        );
        let code = CefString::from(js.as_str());
        let url = CefString::from("");
        frame.execute_java_script(Some(&code), Some(&url), 0);

        let url_str = browser
            .and_then(|b| b.main_frame().map(|f| CefString::from(&f.url()).to_string()))
            .unwrap_or_default();
        tracing::info!(
            "Injected IPC port {} into page: {}",
            self.ipc_port,
            url_str
        );
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
}

// ---------------------------------------------------------------------------
// CefClient — routes to sub-handlers
// ---------------------------------------------------------------------------

wrap_client! {
    pub struct AgentMuxClient {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(AgentMuxDisplayHandler::new(self.inner.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(AgentMuxLifeSpanHandler::new(self.inner.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(AgentMuxLoadHandler::new(self.inner.clone()))
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
            let mut inner = self.inner.lock().expect("Failed to lock handler");
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
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_after_created(browser);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.do_close(browser).into()
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
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
            let mut inner = self.inner.lock().expect("Failed to lock handler");
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
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_load_error(browser, frame, error_code, error_text, failed_url);
        }
    }
}
