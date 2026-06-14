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

                // Startup pool fill — Windows uses the launcher saga path
                // (saga_dispatch.rs::LiveActionRunner is cfg(windows) only).
                //
                // Non-Windows: DISABLED entirely. Two separate blockers, both
                // documented in docs/specs/linux-pool-startup-fill-2026-05-08.md:
                //   1. promote_pool_window in commands/window_pool.rs has a
                //      `cfg(not(target_os = "windows"))` impl that always
                //      returns None — tear-off can't consume a pool window
                //      on macOS or Linux, so any pre-warmed windows are
                //      strictly wasted RAM. Codex P2 on PR #788 caught this
                //      for the macOS path that an earlier revision enabled.
                //   2. (Linux/Wayland only) POOL_OFFSCREEN_X = -32000 is a
                //      Win32/X11 hack that the Wayland compositor ignores —
                //      pool windows would appear on-screen as blank windows.
                //
                // Either blocker alone makes startup pool fill the wrong
                // call here. When the platform pool implementation lands
                // (Phase 7), this is the right place to re-enable.
            }

            // Chrome-style windows (DevTools popups) are shown immediately.
            // Alloy-style windows defer to on_load_end in client.rs to avoid
            // the DWM white flash on startup.
            if self.runtime_style == RuntimeStyle::CHROME {
                window.show();
            }

            // macOS: a non-frameless popup (DevTools) is meant to have a native
            // title bar with a close button (works on Windows), but CEF's
            // Chrome-style window comes up on macOS WITHOUT the standard
            // traffic-light buttons — so DevTools can't be closed with the red X.
            // Force the NSWindow to a standard titled+closable style and un-hide
            // its buttons. The main window is frameless (custom HTML title bar)
            // and is deliberately left untouched.
            #[cfg(target_os = "macos")]
            if !self.frameless {
                let nsview = window.window_handle() as *mut std::ffi::c_void;
                if !nsview.is_null() {
                    unsafe { ensure_macos_native_window_buttons(nsview) };
                }
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
                match browser.host() {
                    Some(host) => host.try_close_browser(),
                    None => 1, // no host yet (pre-init teardown) — allow close
                }
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

/// GPU capability tier, measured once at startup to drive ANGLE backend
/// selection. Precedence: hardware Vulkan → hardware GL → software (SwiftShader).
/// ANGLE is only retargeted, never bypassed. See the GPU block in
/// `on_before_command_line_processing` and
/// docs/specs/SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum GpuTier {
    /// A hardware Vulkan device is present — leave Chromium's default (Vulkan).
    HwVulkan,
    /// No hardware Vulkan but a DRM render node exists — route ANGLE to GL and
    /// override the GPU blocklist (the VMware/SVGA3D case).
    HwGl,
    /// Neither — leave Chromium's default (software SwiftShader).
    Software,
}

#[cfg(target_os = "linux")]
impl GpuTier {
    fn as_str(self) -> &'static str {
        match self {
            GpuTier::HwVulkan => "hw-vulkan",
            GpuTier::HwGl => "hw-gl",
            GpuTier::Software => "software",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "hw-vulkan" => Some(GpuTier::HwVulkan),
            "hw-gl" => Some(GpuTier::HwGl),
            "software" => Some(GpuTier::Software),
            _ => None,
        }
    }
}

/// Resolve the GPU tier once. `on_before_command_line_processing` runs per
/// process; the browser process (which starts first, before any child is
/// spawned) finds `AGENTMUX_GPU_TIER` unset, probes the hardware, and publishes
/// the result. Child processes (gpu/renderer/utility) inherit that env var and
/// read it back — so the `VkInstance` probe runs exactly once, in the browser.
#[cfg(target_os = "linux")]
fn detect_gpu_tier() -> GpuTier {
    if let Ok(v) = std::env::var("AGENTMUX_GPU_TIER") {
        if let Some(t) = GpuTier::from_str(&v) {
            return t;
        }
    }
    let tier = if has_hardware_vulkan() {
        GpuTier::HwVulkan
    } else if has_drm_render_node() {
        GpuTier::HwGl
    } else {
        GpuTier::Software
    };
    // Publish for child processes (inherited through the environment on spawn).
    std::env::set_var("AGENTMUX_GPU_TIER", tier.as_str());
    tracing::info!(tier = tier.as_str(), "resolved GPU tier for ANGLE selection");
    tier
}

/// True if a *hardware* Vulkan device is present. Enumerates Vulkan physical
/// devices and accepts any whose `device_type` is not `CPU` — llvmpipe/lavapipe/
/// SwiftShader all report `CPU`. Fully defensive: any load/create/enumerate
/// failure ⇒ false (we then fall through to the GL check).
#[cfg(target_os = "linux")]
fn has_hardware_vulkan() -> bool {
    use ash::vk;
    let entry = match unsafe { ash::Entry::load() } {
        Ok(e) => e,
        Err(_) => return false,
    };
    let app_info = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_0);
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    let instance = match unsafe { entry.create_instance(&create_info, None) } {
        Ok(i) => i,
        Err(_) => return false,
    };
    let has_hw = unsafe { instance.enumerate_physical_devices() }
        .map(|devices| {
            devices.iter().any(|&d| {
                unsafe { instance.get_physical_device_properties(d) }.device_type
                    != vk::PhysicalDeviceType::CPU
            })
        })
        .unwrap_or(false);
    unsafe { instance.destroy_instance(None) };
    has_hw
}

/// True if a DRM render node (`/dev/dri/renderD*`) exists — a kernel GPU with a
/// render node, i.e. a real hardware GL path (vmwgfx on VMware, i915/amdgpu/
/// nvidia on bare metal). Heuristic; the spec's §7 upgrade path tightens this to
/// a `GL_RENDERER` software-marker check.
#[cfg(target_os = "linux")]
fn has_drm_render_node() -> bool {
    std::fs::read_dir("/dev/dri")
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        })
        .unwrap_or(false)
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

/// macOS: ensure a non-frameless CEF Views window (the DevTools popup) shows the
/// standard title-bar buttons. CEF's Chrome-style popup comes up on macOS without
/// the close/minimize/zoom traffic-lights, so DevTools can't be closed with the X.
/// Reach the NSWindow via `[NSView window]`, set a standard
/// titled+closable+miniaturizable+resizable style mask, and un-hide the three
/// standard buttons. Raw libobjc FFI, mirroring `set_macos_app_display_name`.
#[cfg(target_os = "macos")]
unsafe fn ensure_macos_native_window_buttons(nsview: *mut std::ffi::c_void) {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // nswindow = [nsview window]
    let sel_window = sel_registerName(b"window\0".as_ptr() as _);
    let get_window: extern "C" fn(Id, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = get_window(nsview, sel_window);
    if nswindow.is_null() {
        return;
    }

    // [nswindow setStyleMask: Titled|Closable|Miniaturizable|Resizable]
    // NSWindowStyleMask bits: Titled=1, Closable=2, Miniaturizable=4, Resizable=8.
    let sel_set_mask = sel_registerName(b"setStyleMask:\0".as_ptr() as _);
    let set_mask: extern "C" fn(Id, Sel, usize) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_mask(nswindow, sel_set_mask, 1 | 2 | 4 | 8);

    // Un-hide the standard window buttons (close=0, miniaturize=1, zoom=2).
    let sel_std_btn = sel_registerName(b"standardWindowButton:\0".as_ptr() as _);
    let std_btn: extern "C" fn(Id, Sel, usize) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let sel_set_hidden = sel_registerName(b"setHidden:\0".as_ptr() as _);
    let set_hidden: extern "C" fn(Id, Sel, u8) =
        std::mem::transmute(objc_msgSend as *const c_void);
    for btn_kind in [0usize, 1, 2] {
        let btn = std_btn(nswindow, sel_std_btn, btn_kind);
        if !btn.is_null() {
            set_hidden(btn, sel_set_hidden, 0);
        }
    }

    tracing::info!("macOS: forced native title-bar buttons on non-frameless popup (DevTools)");
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
                //
                // Also disable MediaRouter: Chromium's Cast/DIAL device
                // discovery probes the local network (mDNS/SSDP), which trips
                // the macOS "AgentMux would like to find devices on your local
                // network" prompt at launch. AgentMux never casts, so this is
                // pure overhead — disabling it keeps the app off the local
                // network entirely unless the user explicitly turns on LAN
                // instance discovery (`network:lan_discovery`, off by default
                // and lazily started). See feedback_no_os_notices_at_launch.
                // Disabled features (comma-joined; one switch — a second
                // --disable-features would override the first):
                //   CalculateNativeWinOcclusion — empty browser on visibility (CEF #3638)
                //   MediaRouter                  — Cast/DIAL local-network discovery (TCC prompt)
                //   PreconnectToSearch           — Chromium warms the default search engine
                //                                  (Google) at startup → www/accounts.google.com
                //   AutofillServerCommunication  — Autofill field-metadata downloads
                //                                  → content-autofill.googleapis.com
                // The last two were the actual Google QUIC traffic observed via
                // net-log; AgentMux makes no such calls itself.
                let key = CefString::from("disable-features");
                // MachPortRendezvous{Validate,Enforce}PeerRequirements: belt-and-
                // suspenders flags kept for defence in depth. NOTE: the actual fix
                // for the macOS-26 renderer crash (-67030 / errSecCSReqFailed) is
                // the source patch in docs/cef-patches/ (GetPeerValidationPolicy →
                // kNoValidation) — the policy is read before FeatureList init so
                // this runtime flag cannot apply in time and is NOT the load-bearing
                // mechanism. Do not remove the patch thinking these flags cover it.
                let val = CefString::from(
                    "CalculateNativeWinOcclusion,MediaRouter,PreconnectToSearch,\
                     AutofillServerCommunication,MachPortRendezvousValidatePeerRequirements,\
                     MachPortRendezvousEnforcePeerRequirements",
                );
                cmd.append_switch_with_value(Some(&key), Some(&val));

                // ── Linux: default to XWayland (X11 ozone) ─────────────────
                // Native-Wayland Ozone in CEF 146/Chromium 146 has broken
                // Mutter↔Chromium GPU buffer negotiation
                // (`WaylandZwpLinuxDmabuf::OnTrancheFlags Not implemented`
                // at startup), which causes the cc::Scheduler to reply
                // `LayerTreeHostImpl::DidNotProduceFrame` to ~89 % of
                // Mutter's `BeginFrame` requests. With BeginMainFrame
                // production stuck near sysinfo's ~1 Hz invalidation
                // cadence, the renderer's `requestAnimationFrame`
                // callbacks (including predictive local echo's render
                // path, #1223) fire only when sysinfo dirties the status
                // bar — typing visibly hangs and pumps out on key release.
                //
                // Measured locally with `scripts/capture-trace.cjs` +
                // CDP `Profiler.start`:
                //
                //   |                 | Wayland | XWayland |
                //   | --------------- | ------- | -------- |
                //   | rAF firing rate | 2.5 Hz  | 6.4 Hz   |
                //   | rAF gap p95     | 1182 ms | 224 ms   |
                //   | rAF gap max     | 8280 ms | 1024 ms  |
                //
                // XWayland is on the well-trodden X11 present path —
                // 5–8× fewer stalls on the wire-format Linux Chromium has
                // shipped reliably for years. Future CEF 148 + a forward-
                // ported Wayland-aware libcef.so may make native Wayland
                // viable; keep XWayland as the default until then. Opt
                // out via `AGENTMUX_OZONE_PLATFORM=wayland` for native-
                // Wayland regression testing.
                #[cfg(target_os = "linux")]
                {
                    let ozone_choice = std::env::var("AGENTMUX_OZONE_PLATFORM")
                        .ok()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "x11".to_string());
                    let oz_key = CefString::from("ozone-platform");
                    let oz_val = CefString::from(ozone_choice.as_str());
                    cmd.append_switch_with_value(Some(&oz_key), Some(&oz_val));
                }

                // ── ANGLE backend precedence (capability-probed) ─────────────
                // Precedence: hardware Vulkan → hardware GL → software
                // (SwiftShader). CEF 148 defaults ANGLE to Vulkan and accepts a
                // *software* Vulkan ICD over a perfectly good hardware-GL path —
                // exactly the VMware case (no HW Vulkan, but HW SVGA3D GL), where
                // the result is SwiftShader: burst-paint terminals + no WebGL. We
                // measure the GPU tier and retarget ANGLE accordingly; ANGLE is
                // never bypassed, only pointed at the best real backend. On the
                // HW-GL rung we also --ignore-gpu-blocklist (Chromium blocklists
                // the virtual GPU it just lost Vulkan on). No vendor gate: VMware
                // simply lands in the HW-GL rung by measurement, real GPUs stay
                // on Vulkan, headless stays software. Win/macOS report hardware
                // and land in the top rung untouched.
                // Spec: docs/specs/SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md
                //
                // Authority: explicit AGENTMUX_ANGLE env > measured precedence >
                // Chromium default. AGENTMUX_CEF_EXTRA_FLAGS appends switches.
                let angle_override = std::env::var("AGENTMUX_ANGLE")
                    .ok()
                    .filter(|s| !s.is_empty());

                #[cfg(target_os = "linux")]
                let (angle, ignore_blocklist): (Option<String>, bool) = match angle_override {
                    Some(a) => (Some(a), false),
                    None => match detect_gpu_tier() {
                        GpuTier::HwVulkan => (None, false),
                        GpuTier::HwGl => (Some("gl".to_string()), true),
                        GpuTier::Software => (None, false),
                    },
                };
                #[cfg(not(target_os = "linux"))]
                let (angle, ignore_blocklist): (Option<String>, bool) = (angle_override, false);

                if let Some(angle) = angle {
                    cmd.append_switch_with_value(
                        Some(&CefString::from("use-angle")),
                        Some(&CefString::from(angle.as_str())),
                    );
                }
                if ignore_blocklist {
                    cmd.append_switch(Some(&CefString::from("ignore-gpu-blocklist")));
                }

                // ── Arbitrary extra Chromium switches (dev/diagnostics) ───────
                // AGENTMUX_CEF_EXTRA_FLAGS lets us A/B GPU/compositor flags
                // without a recompile: space-separated tokens, each either
                // `--switch`, `--switch=value`, or `switch=value`. Used to hunt
                // the WebGL / gpu-compositing gate on VM guests. Empty/unset → no-op.
                if let Ok(extra) = std::env::var("AGENTMUX_CEF_EXTRA_FLAGS") {
                    for tok in extra.split_whitespace() {
                        let tok = tok.trim_start_matches("--");
                        if tok.is_empty() {
                            continue;
                        }
                        if let Some((k, v)) = tok.split_once('=') {
                            cmd.append_switch_with_value(
                                Some(&CefString::from(k)),
                                Some(&CefString::from(v)),
                            );
                        } else {
                            cmd.append_switch(Some(&CefString::from(tok)));
                        }
                    }
                }

                // ── GPU channel + frame-production tuning ─────────────────────
                let gpu_features_key = CefString::from("enable-features");
                let gpu_features_val = CefString::from(
                    "EarlyEstablishGpuChannel,EstablishGpuChannelAsync",
                );
                cmd.append_switch_with_value(
                    Some(&gpu_features_key),
                    Some(&gpu_features_val),
                );

                // Stop Chromium's background phone-home — the QUIC connections
                // to Google (1e100.net) that an embedded CEF app neither needs
                // nor should make. `disable-background-networking` covers most
                // background subsystems; the rest get an explicit switch because
                // Chromium leaves them on otherwise.
                cmd.append_switch(Some(&CefString::from("disable-background-networking")));
                cmd.append_switch(Some(&CefString::from("disable-component-update")));
                cmd.append_switch(Some(&CefString::from("disable-domain-reliability")));
                cmd.append_switch(Some(&CefString::from("disable-field-trial-config")));
                let vurl = CefString::from("variations-server-url");
                let vempty = CefString::from("");
                cmd.append_switch_with_value(Some(&vurl), Some(&vempty));

                // Initial background color, ARGB hex. alpha=00 → fully
                // transparent → first-frame paint is alpha-aware so the
                // CSS body background's rgba() composes with the desktop
                // wallpaper. ff222222 here would clobber the alpha=0 we set
                // via CefSettings.background_color in main.rs and force the
                // first frame opaque (visible as a brief flash even after
                // the renderer flips to ARGB on the first commit).
                // Pair with: main.rs CefSettings.background_color = 0,
                // app.rs BrowserSettings.background_color = 0, and the
                // is_frameless main window delegate.
                let bg_key = CefString::from("background-color");
                let bg_val = CefString::from("00000000");
                cmd.append_switch_with_value(Some(&bg_key), Some(&bg_val));

                // Disable LCD text rendering — LCD subpixel anti-aliasing
                // requires opaque backgrounds, so Chromium force-sets
                // contents_opaque=true on every compositor layer that contains
                // LCD-rendered text. With opaque layers, even CSS alpha<1
                // backgrounds get rasterized as fully opaque, defeating the
                // whole transparency cascade. Grayscale text AA on a
                // translucent UI is the standard tradeoff for window
                // transparency.
                let lcd_key = CefString::from("disable-lcd-text");
                cmd.append_switch(Some(&lcd_key));

                // Allow the DevTools inspector page (served from the remote
                // debugging server) to open its own WebSocket connection back
                // to that same server.  Without this flag Chromium 107+ blocks
                // cross-origin WebSocket upgrades to the debug port.
                let ro_key = CefString::from("remote-allow-origins");
                let ro_val = CefString::from("*");
                cmd.append_switch_with_value(Some(&ro_key), Some(&ro_val));

                // Skip Chrome features that add startup latency with no
                // user-visible benefit in this app.
                //
                // `--no-proxy-server` was previously included here to skip
                // WPAD/PAC auto-detect (2–3 s cold-start hit). Removed
                // because it disables proxy support GLOBALLY — the
                // `browser` widget loads arbitrary external URLs and
                // would break for users on corporate networks where
                // outbound HTTP requires the configured proxy. A future
                // optimization could disable WPAD only without
                // disabling explicit proxy config.
                cmd.append_switch(Some(&CefString::from("disable-sync")));
                cmd.append_switch(Some(&CefString::from("disable-extensions")));

                // HTTP Basic / Digest auth — route the challenge to the
                // embedder's `RequestHandler::on_auth_credentials` callback
                // (which surfaces our `BrowserAuthModal` from PR #906)
                // instead of letting the Chrome runtime show its own in-
                // process login dialog. Without this switch, CEF 146
                // silently consumes the challenge and the request fails
                // with `ERR_INVALID_AUTH_CREDENTIALS (-338)` — the
                // embedder callback is never invoked, so the auth modal
                // stays dormant.
                //
                // The Alloy runtime was removed in 2024, so CEF 146 only
                // ships the Chrome runtime — this switch is mandatory.
                // CEF4Delphi sets it by default for the same reason
                // (sister binding hitting the same surface).
                //
                // Tracking: docs/research/
                //   RESEARCH_CEF_AUTH_CALLBACK_SUPPRESSED_2026_05_25.md
                // Upstream: https://github.com/chromiumembedded/cef/issues/3603
                cmd.append_switch(Some(&CefString::from("disable-chrome-login-prompt")));

                // Never request OS credential / keychain access — in ANY
                // runtime mode (dev, installed, portable), on any platform.
                // These switches route Chromium's OSCrypt away from the
                // platform-native credential store, which otherwise pops an
                // "AgentMux wants to use your confidential information stored
                // in Login" prompt the first time it's touched — and on macOS
                // that modal BLOCKS browser startup, so the app hangs with no
                // window. AgentMux never saves a password, so OSCrypt only
                // matters for the cookie jar; for a local single-user
                // workbench (data dir 0700) an obfuscation-key store is an
                // acceptable tradeoff vs prompting the user for keychain
                // access. This reverses the earlier "prompt is appropriate for
                // released builds" gating. See
                // docs/specs/SPEC_SUPPRESS_OS_CREDENTIAL_PROMPTS_2026_05_30.md
                // and docs/retro/retro-macos-keychain-prompt-2026-05-30.md.

                // `--password-store=basic` swaps Chromium's password backend
                // from the platform-native store (Keychain on macOS,
                // gnome-keyring/kwallet on Linux, CredVault on Windows) to an
                // in-process basic store — no native-backend prompt.
                let ps_key = CefString::from("password-store");
                let ps_val = CefString::from("basic");
                cmd.append_switch_with_value(Some(&ps_key), Some(&ps_val));

                // macOS belt-and-suspenders: even with password-store=basic,
                // OSCrypt still fetches its encryption key from the Keychain
                // unless the keychain itself is mocked. `--use-mock-keychain`
                // redirects those calls to an in-process mock. macOS-only.
                #[cfg(target_os = "macos")]
                cmd.append_switch(Some(&CefString::from("use-mock-keychain")));

                // GPU compositing runs in a separate process (Chromium default).
                // This allows Chromium to restart the GPU process transparently
                // after driver resets (TDR, DXGI device removal, display power
                // state changes). The ~100GB VA overhead is virtual, not physical
                // (~20-50MB RSS), and negligible on 64-bit systems.
                //
                // Previously used --in-process-gpu to save VA space, but it left
                // the app in a zombie white-screen state on GPU context loss with
                // no recovery path. Removed in v0.33.66.

                // Software-GL safety net. Chromium 110+ gates the SwiftShader
                // fallback for WebGL behind this switch. When the hardware GPU
                // process can't boot (e.g. the Windows STATUS_BREAKPOINT-at-init
                // crash from the DCHECK-enabled libcef build, or any driver/virtual-
                // display failure), this + the bundled vk_swiftshader.dll/vulkan-1.dll
                // (Taskfile bundle) let GL degrade to *software* WebGL instead of
                // being disabled entirely — keeping the xterm WebGL renderer (and its
                // scrollbar) working. Hardware GL is still preferred when available,
                // so there is no cost on healthy machines. This is a fallback only;
                // the real fix is an official (DCHECK-off) Windows libcef build.
                cmd.append_switch(Some(&CefString::from("enable-unsafe-swiftshader")));

                // NOTE: `--renderer-process-limit=1` was previously set here to
                // protect against DevTools popups spawning extra renderers under
                // an Alloy-mode assumption. The current Linux CEF build is NOT
                // Alloy-mode for the user-visible UI: main window, every pool
                // window, every tear-off window, and every browser-pane gets
                // its own renderer process. Capping all of them to ONE shared
                // renderer process serializes their JS event loops on a single
                // thread, which manifests as hover/animation lag in the user-
                // visible UI when pool windows are doing idle work. Removed
                // 2026-05-09. See docs/specs/linux-cef-flags-audit-2026-05-08.md.
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
                // ARGB: alpha=0 → SK_AlphaTRANSPARENT → enables Views-framework
                // transparency in the patched libcef.so. Pair with the
                // CefSettings::background_color flip in main.rs and the
                // is_frameless=true main window delegate. See
                // docs/research/cef-transparency-research-2026-05-10.md and
                // docs/retros/cef-transparency-empirical-2026-05-11.md.
                background_color: 0x00000000,
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
                // Use the launcher's mode if it set the env, else
                // fall back to detecting from the host exe path
                // (covers standalone `task dev` runs).
                let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                        .map(|d| agentmux_common::RuntimeMode::current(&d))
                });
                let is_dev = matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. }));
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
                    // Dev mode only: Vite HMR server. Honor AGENTMUX_VITE_PORT
                    // (per-clone port from Taskfile.yml dev:serve); see
                    // docs/analyses/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md.
                    let port: u16 = std::env::var("AGENTMUX_VITE_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(5173);
                    format!("http://localhost:{}", port)
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
