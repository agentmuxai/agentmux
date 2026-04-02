// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefApp and BrowserProcessHandler implementations for AgentMux CEF host.
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

wrap_window_delegate! {
    pub struct AgentMuxWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
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

            // Resize to 70% of the current monitor's work area, centered.
            if let Some((x, y, w, h)) = get_monitor_centered_70pct(window) {
                window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
            }

            window.show();
            // Focus the browser so keyboard input (Ctrl+C/V, typing) works immediately.
            if let Some(browser) = browser_view.browser() {
                if let Some(host) = browser.host() {
                    host.set_focus(1);
                }
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            let mut browser_view = self.browser_view.borrow_mut();
            *browser_view = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view = self.browser_view.borrow();
            let browser_view = browser_view.as_ref().expect("BrowserView is None");
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
            1 // Frameless — AgentMux uses its own custom title bar
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
            RuntimeStyle::ALLOY
        }
    }
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
        let rc = info.rcWork;
        Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
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
            _is_devtools: i32,
        ) -> i32 {
            // Create a new top-level window for popups (e.g., devtools).
            let mut window_delegate = AgentMuxWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
            );
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
                ));
            }

            // Browser settings.
            let settings = BrowserSettings {
                windowless_frame_rate: 60,
                // Dark background to match app theme — prevents white bleed-through
                // when terminal panes use transparency.
                background_color: 0xFF000000, // ARGB: opaque black
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
                let is_dev = std::env::var("AGENTMUX_DEV").is_ok();
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

            // Default: native window mode to eliminate white flash.
            // Window starts hidden (no WS_VISIBLE), shown in on_load_end.
            // Pass --use-views to use CEF Views instead (has white flash).
            let use_views = command_line.has_switch(Some(&CefString::from("use-views"))) != 0;

            if !use_views {
                #[cfg(target_os = "windows")]
                let window_info = {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let (x, y, w, h) = get_monitor_work_area(0, 0)
                        .map(|(wx, wy, ww, wh)| {
                            let w = (ww as f64 * 0.70) as i32;
                            let h = (wh as f64 * 0.70) as i32;
                            (wx + (ww - w) / 2, wy + (wh - h) / 2, w, h)
                        })
                        .unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT, 1200, 800));
                    WindowInfo {
                        runtime_style: RuntimeStyle::ALLOY,
                        window_name: CefString::from("AgentMux"),
                        // No WS_VISIBLE, no WS_THICKFRAME at creation — both
                        // cause white flash. WS_THICKFRAME added in on_load_end
                        // after content paints (enables resize without flash).
                        style: WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
                            | WS_MINIMIZEBOX | WS_MAXIMIZEBOX,
                        bounds: cef::Rect { x, y, width: w, height: h },
                        ..Default::default()
                    }
                };
                #[cfg(not(target_os = "windows"))]
                let window_info = WindowInfo {
                    runtime_style: RuntimeStyle::ALLOY,
                    ..Default::default()
                };

                let mut client = self.default_client();
                browser_host_create_browser(
                    Some(&window_info),
                    client.as_mut(),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                );
            } else {
                // CEF Views mode (--use-views): cross-platform but has white flash.
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
                );
                window_create_top_level(Some(&mut window_delegate));
            }
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}
