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

        // Register browser in the multi-window map.
        // First browser is "main"; additional browsers get labels from their URL params.
        {
            let mut browsers = self.state.browsers.lock().unwrap();
            let label = if browsers.is_empty() {
                "main".to_string()
            } else {
                // Extract windowLabel from the URL query params if available
                let url = browser.main_frame()
                    .map(|f| { let u = f.url(); CefString::from(&u).to_string() })
                    .unwrap_or_default();
                extract_query_param(&url, "windowLabel")
                    .unwrap_or_else(|| format!("window-{}", uuid::Uuid::new_v4()))
            };
            tracing::info!("Registered browser: label={} (total: {})", label, browsers.len() + 1);
            browsers.insert(label, browser.clone());
        }

        // For ALL native windows: extend the client area into the frame
        // to hide the visible WS_THICKFRAME resize border.
        // For SECONDARY windows only: install WM_NCHITTEST hook for edge resize
        // and show the window (created hidden to avoid white-border flash).
        // The main window (CEF Views) handles resize via its delegate — we must
        // NOT install the WndProc hook on it or it breaks CEF's GWLP_USERDATA.
        #[cfg(target_os = "windows")]
        {
            let is_secondary = self.browser_list.len() > 0; // first browser not yet pushed
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    unsafe {
                        setup_native_frameless(hwnd.0 as *mut std::ffi::c_void);
                        if is_secondary {
                            install_frameless_resize_hook(hwnd.0 as *mut std::ffi::c_void);
                            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOW};
                            ShowWindow(hwnd.0 as _, SW_SHOW);
                        }
                    }
                }
            }
        }

        // Set the taskbar/title bar icon from the embedded exe resource.
        #[cfg(target_os = "windows")]
        {
            // For native windows, use the browser's HWND; for CEF Views, enumerate.
            let hwnd = browser.host()
                .and_then(|h| {
                    let wh = h.window_handle();
                    if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                })
                .unwrap_or_else(|| unsafe {
                    crate::commands::window::find_own_top_level_window()
                });
            if !hwnd.is_null() {
                unsafe { set_window_icon(hwnd); }
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

        // Unregister browser from the multi-window map.
        {
            let mut browsers = self.state.browsers.lock().unwrap();
            let label = browsers.iter()
                .find(|(_, b)| b.is_same(Some(&mut browser)) != 0)
                .map(|(k, _)| k.clone());
            if let Some(label) = label {
                browsers.remove(&label);
                tracing::info!("Unregistered browser: label={} (remaining: {})", label, browsers.len());
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

        let ipc_token = &self.state.ipc_token;
        let js = format!(
            "window.__AGENTMUX_IPC_PORT__ = {}; window.__AGENTMUX_IPC_TOKEN__ = '{}';",
            self.ipc_port, ipc_token
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
    ORIGINAL_WNDPROCS
        .lock()
        .unwrap()
        .insert(hwnd as usize, original);
    SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as isize);
    tracing::info!("Installed frameless resize hook (WM_NCCALCSIZE + WM_NCHITTEST)");
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

/// Extract a query parameter value from a URL string.
fn extract_query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            return kv.next().map(|v| v.to_string());
        }
    }
    None
}
