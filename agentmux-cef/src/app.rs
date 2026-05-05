// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefApp and BrowserProcessHandler implementations for AgentMux host.
// Creates a browser window loading the frontend URL on context initialization.
//
// Phase 2: Stores AppState and injects IPC port into the page after load.

use cef::*;
use std::cell::RefCell;
use std::sync::Arc;

use crate::client::*;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Window & BrowserView delegates (CEF Views framework)
// ---------------------------------------------------------------------------

// Linux/macOS only: when `Some((state, window_label))`, `on_window_created`
// inserts the Window into `state.windows[window_label]` and
// `on_window_destroyed` removes it. Browser-pane creation
// (`browser_pane/creation_views.rs`) looks up the parent Window by label
// to call `add_overlay_view` on. Without this, panes opened from a
// non-main window were silently routed to the main window. Popup
// delegates (DevTools etc.) pass `None` and don't register because they
// shouldn't host user-facing panes.
// (kept as a regular comment — wrap_window_delegate! doesn't accept
// doc-comments on struct fields.)
wrap_window_delegate! {
    pub struct AgentMuxWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        initial_bounds: Option<(i32, i32, i32, i32)>,
        frameless: bool,
        runtime_style: RuntimeStyle,
        window_registration: Option<(Arc<AppState>, String)>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size {
                width: 1200,
                height: 800,
            }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));

            // Position: use explicit bounds if provided, else 70% centered.
            if let Some((x, y, w, h)) = self.initial_bounds {
                window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
            } else if let Some((x, y, w, h)) = get_monitor_centered_70pct(window) {
                window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
            }

            // Linux/macOS only — register this Window in state.windows
            // keyed by label, so the browser-pane Views path can attach
            // pane overlays to the right window. Popup delegates pass
            // `None` and don't register (they shouldn't host panes).
            #[cfg(not(target_os = "windows"))]
            if let Some((state, label)) = self.window_registration.as_ref() {
                state.windows.lock().insert(label.clone(), window.clone());
                tracing::info!(
                    window_label = %label,
                    "[browser-pane] registered Window in state.windows for pane attachment"
                );
            }

            // Chrome-style windows (DevTools popups) are shown immediately.
            // Alloy-style windows defer to on_load_end in client.rs to avoid
            // the DWM white flash on startup.
            if self.runtime_style == RuntimeStyle::CHROME {
                window.show();
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            let mut browser_view = self.browser_view.borrow_mut();
            *browser_view = None;

            // Linux/macOS — un-register this Window from state.windows.
            // Stale entries would cause subsequent pane creates targeting
            // a destroyed window to silently no-op or worse.
            #[cfg(not(target_os = "windows"))]
            if let Some((state, label)) = self.window_registration.as_ref() {
                state.windows.lock().remove(label);
                tracing::info!(
                    window_label = %label,
                    "[browser-pane] unregistered Window on destroy"
                );
            }
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view = self.browser_view.borrow();
            let Some(browser_view) = browser_view.as_ref() else {
                return 1;
            };
            if let Some(browser) = browser_view.browser() {
                let browser_host = browser.host().expect("BrowserHost is None");
                browser_host.try_close_browser()
            } else {
                1
            }
        }

        fn initial_show_state(&self, _window: Option<&mut Window>) -> ShowState {
            ShowState::NORMAL
        }

        fn is_frameless(&self, _window: Option<&mut Window>) -> i32 {
            self.frameless as i32
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }

        // Wayland app_id / X11 WM_CLASS are set via an FFI override below
        // (see install_linux_window_properties_override) instead of via this
        // trait method, because the cef 146.7.0 wrapper's
        // `From<CefStringUtf16> for _cef_string_utf16_t` impl silently drops
        // `Clear` variants — the kind `CefString::from("agentmux")` produces.
        // The trait method would set the values, the writeback would zero
        // them, and CEF would emit `xdg_toplevel.set_app_id("")`.
    }
}

/// Override the `get_linux_window_properties` function pointer on a
/// `WindowDelegate` to write the AgentMux app_id directly to the C struct,
/// bypassing the buggy `CefString` → `cef_string_utf16_t` conversion in the
/// cef 146.7.0 wrapper (`Clear` variant gets dropped during writeback).
///
/// Without this, CEF emits `xdg_toplevel.set_app_id("")` and GNOME / KWin /
/// sway can't match the window to `agentmux.desktop`, so the AgentMux icon
/// never appears in the taskbar/dock/launcher.
///
/// Must be called once on every `WindowDelegate` we create (top-level, popup,
/// new sub-window) before passing it to `window_create_top_level`.
#[cfg(target_os = "linux")]
pub fn install_linux_window_properties_override(delegate: &cef::WindowDelegate) {
    use cef::ImplWindowDelegate;
    // Disambiguate: WindowDelegate implements get_raw on three traits
    // (ImplViewDelegate / ImplPanelDelegate / ImplWindowDelegate). We need
    // the WindowDelegate one to get the right struct type for casting.
    let raw: *mut cef::sys::_cef_window_delegate_t =
        <cef::WindowDelegate as ImplWindowDelegate>::get_raw(delegate);
    unsafe {
        (*raw).get_linux_window_properties = Some(write_linux_window_properties);
    }
}

/// Custom extern "C" shim invoked by libcef to populate
/// `_cef_linux_window_properties_t`. Writes "agentmux" to wayland_app_id
/// and the X11 wm_class fields via cef-dll-sys utf8→utf16 setters,
/// then returns 1 so libcef uses the values.
#[cfg(target_os = "linux")]
extern "C" fn write_linux_window_properties(
    _self_: *mut cef::sys::_cef_window_delegate_t,
    _window: *mut cef::sys::_cef_window_t,
    properties: *mut cef::sys::_cef_linux_window_properties_t,
) -> std::os::raw::c_int {
    if properties.is_null() {
        return 0;
    }
    const APP_ID: &[u8] = b"agentmux";
    unsafe {
        let props = &mut *properties;
        // The C struct's strings start zeroed (libcef constructs a default
        // CefLinuxWindowProperties). cef_string_utf8_to_utf16 allocates a
        // new utf-16 buffer and assigns it to the dest cef_string_utf16_t;
        // ownership transfers to libcef which calls dtor when done.
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wayland_app_id,
        );
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wm_class_class,
        );
        cef::sys::cef_string_utf8_to_utf16(
            APP_ID.as_ptr().cast(), APP_ID.len(), &mut props.wm_class_name,
        );
    }
    1
}

/// Compute a centered 70% rect for the monitor the window is currently on.
/// Returns (x, y, width, height) or None if the monitor can't be determined.
fn get_monitor_centered_70pct(window: &Window) -> Option<(i32, i32, i32, i32)> {
    let bounds = window.bounds();
    let (work_x, work_y, work_w, work_h) = get_monitor_work_area(bounds.x, bounds.y)?;
    let w = (work_w as f64 * 0.70) as i32;
    let h = (work_h as f64 * 0.70) as i32;
    let x = work_x + (work_w - w) / 2;
    let y = work_y + (work_h - h) / 2;
    Some((x, y, w, h))
}

/// Get the work area (excluding taskbar/dock) of the monitor containing (px, py).
/// Returns (x, y, width, height) of the work area.
#[cfg(target_os = "windows")]
pub fn get_monitor_work_area(px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Graphics::Gdi::{
        MonitorFromPoint, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let point = windows_sys::Win32::Foundation::POINT { x: px, y: py };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
        if hmonitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info) == 0 {
            return None;
        }
        // Convert physical pixels → DIP (logical) pixels.
        // CEF Views set_bounds() expects DIP; GetMonitorInfoW returns physical pixels.
        // On Windows 10 @ 100%: dpi_x == 96 → scale == 1.0 (no change).
        // On Windows 11 @ 125%: dpi_x == 120 → divide physical coords by 1.25.
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f64 / 96.0;
        let rc = info.rcWork;
        Some((
            (rc.left as f64 / scale).round() as i32,
            (rc.top as f64 / scale).round() as i32,
            ((rc.right - rc.left) as f64 / scale).round() as i32,
            ((rc.bottom - rc.top) as f64 / scale).round() as i32,
        ))
    }
}

#[cfg(target_os = "macos")]
pub fn get_monitor_work_area(_px: i32, _py: i32) -> Option<(i32, i32, i32, i32)> {
    // TODO: Use NSScreen.main.visibleFrame for proper work area (minus Dock/menu bar).
    // CGMainDisplayID only returns the primary display — doesn't support multi-monitor
    // and hardcoding menu bar height is fragile. Fall back to 1200x800 default.
    None
}

#[cfg(target_os = "linux")]
pub fn get_monitor_work_area(_px: i32, _py: i32) -> Option<(i32, i32, i32, i32)> {
    // X11: XDisplayWidth/XDisplayHeight on the default screen.
    // This is the full screen, not work area (no taskbar subtraction).
    // TODO: use _NET_WORKAREA from the root window for proper work area.
    None // Falls back to 1200x800 default
}

wrap_browser_view_delegate! {
    pub struct AgentMuxBrowserViewDelegate {
        runtime_style: RuntimeStyle,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            is_devtools: i32,
        ) -> i32 {
            // Create a new top-level window for the popup.
            // DevTools windows (is_devtools != 0) get a native title bar so the
            // user can see it's DevTools, move it, and close it with the X button.
            // Regular popups stay frameless (matching the main window style).
            let frameless = is_devtools == 0;
            // DevTools popups are always Chrome-style (even from Alloy parents).
            // The window runtime style must match the browser view style or CEF crashes.
            let runtime_style = if is_devtools != 0 {
                RuntimeStyle::CHROME
            } else {
                RuntimeStyle::ALLOY
            };
            let mut window_delegate = AgentMuxWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
                None,
                frameless,
                runtime_style,
                None, // popup (DevTools etc.) — don't register; not pane-host
            );
            #[cfg(target_os = "linux")]
            install_linux_window_properties_override(&window_delegate);
            window_create_top_level(Some(&mut window_delegate));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }
    }
}

// ---------------------------------------------------------------------------
// CefApp + BrowserProcessHandler
// ---------------------------------------------------------------------------

wrap_app! {
    pub struct AgentMuxApp {
        state: Arc<AppState>,
        ipc_port: u16,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                // Prevent empty browser on visibility change (CEF #3638).
                let key = CefString::from("disable-features");
                let val = CefString::from("CalculateNativeWinOcclusion");
                cmd.append_switch_with_value(Some(&key), Some(&val));

                // Set initial background color via CLI.
                let bg_key = CefString::from("background-color");
                let bg_val = CefString::from("ff222222");
                cmd.append_switch_with_value(Some(&bg_key), Some(&bg_val));

                // Allow the DevTools inspector page (served from the remote
                // debugging server) to open its own WebSocket connection back
                // to that same server.  Without this flag Chromium 107+ blocks
                // cross-origin WebSocket upgrades to the debug port.
                let ro_key = CefString::from("remote-allow-origins");
                let ro_val = CefString::from("*");
                cmd.append_switch_with_value(Some(&ro_key), Some(&ro_val));

                // GPU compositing runs in a separate process (Chromium default).
                // This allows Chromium to restart the GPU process transparently
                // after driver resets (TDR, DXGI device removal, display power
                // state changes). The ~100GB VA overhead is virtual, not physical
                // (~20-50MB RSS), and negligible on 64-bit systems.
                //
                // Previously used --in-process-gpu to save VA space, but it left
                // the app in a zombie white-screen state on GPU context loss with
                // no recovery path. Removed in v0.33.66.

                // Cap renderer subprocesses. In Alloy mode the frontend runs
                // in the browser process (no renderer spawned), but DevTools
                // popups can spawn additional renderers at ~100GB VA each.
                let rpl_key = CefString::from("renderer-process-limit");
                let rpl_val = CefString::from("1");
                cmd.append_switch_with_value(Some(&rpl_key), Some(&rpl_val));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AgentMuxBrowserProcessHandler::new(
                RefCell::new(None),
                self.state.clone(),
                self.ipc_port,
            ))
        }
    }
}

// AgentMuxApp::new(state, ipc_port) is generated by the wrap_app! macro above.

wrap_browser_process_handler! {
    pub struct AgentMuxBrowserProcessHandler {
        client: RefCell<Option<Client>>,
        state: Arc<AppState>,
        ipc_port: u16,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            // Create the client (browser-level callbacks) with state for IPC port injection.
            {
                let mut client = self.client.borrow_mut();
                *client = Some(AgentMuxClient::new(
                    AgentMuxHandler::new(self.state.clone(), self.ipc_port),
                    false, // is_browser_pane = false — main browser takes focus normally
                ));
            }

            // Browser settings.
            let settings = BrowserSettings {
                windowless_frame_rate: 60,
                // Dark background to match app theme — prevents white bleed-through
                // when terminal panes use transparency.
                background_color: 0xFF000000, // ARGB: opaque black (matches pre-transparency-experiment baseline)
                ..Default::default()
            };

            // Determine the URL to load.
            let command_line = command_line_get_global().expect("Failed to get command line");
            let url_switch = CefString::from("url");
            let base_url = if command_line.has_switch(Some(&url_switch)) != 0 {
                CefString::from(&command_line.switch_value(Some(&url_switch))).to_string()
            } else {
                String::new()
            };
            // If no URL specified, load from the IPC server (which serves static
            // files from the bundled frontend). Fall back to Vite dev server ONLY
            // in dev mode — in release builds, localhost:5173 doesn't exist and
            // would show a raw browser error page.
            let base_url = if base_url.is_empty() {
                let is_dev = matches!(
                    agentmux_common::RuntimeMode::from_env(),
                    Some(agentmux_common::RuntimeMode::Dev { .. })
                );
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                let has_frontend = exe_dir
                    .as_ref()
                    .map(|d| d.join("frontend/index.html").exists())
                    .unwrap_or(false);
                if has_frontend || !is_dev {
                    // Production or portable: always use IPC server
                    format!("http://127.0.0.1:{}", self.ipc_port)
                } else {
                    // Dev mode only: Vite HMR server
                    "http://localhost:5173".to_string()
                }
            } else {
                base_url
            };

            // Append IPC port and token as URL query parameters so the frontend
            // can detect CEF mode and connect to the IPC server immediately,
            // before on_load_end fires.
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let url_with_ipc = format!(
                "{}{}ipc_port={}&ipc_token={}",
                base_url, separator, self.ipc_port, self.state.ipc_token
            );
            let url = CefString::from(url_with_ipc.as_str());

            tracing::info!("Loading URL: {}{}ipc_port={}&ipc_token=<redacted>", base_url, separator, self.ipc_port);

            // CEF Views mode — window NOT shown until on_load_end.
            // No DwmExtendFrameIntoClientArea (causes white flash).
            // CEF Views handles resize, snap, frameless natively.
            {
                let mut client = self.default_client();
                let mut delegate = AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY);
                let browser_view = browser_view_create(
                    client.as_mut(),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                    Some(&mut delegate),
                );

                let mut window_delegate = AgentMuxWindowDelegate::new(
                    RefCell::new(browser_view),
                    None,
                    true, // frameless — main window uses custom title bar
                    RuntimeStyle::ALLOY,
                    Some((self.state.clone(), "main".to_string())),
                );
                #[cfg(target_os = "linux")]
                install_linux_window_properties_override(&window_delegate);
                window_create_top_level(Some(&mut window_delegate));
            }
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}
