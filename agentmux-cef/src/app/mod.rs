// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefApp and BrowserProcessHandler implementations for AgentMux host.
// Creates a browser window loading the frontend URL on context initialization.
//
// Phase 2: Stores AppState and injects IPC port into the page after load.
//
// This module holds the irreducible CEF FFI boilerplate: the
// `wrap_window_delegate!`/`wrap_browser_view_delegate!`/`wrap_app!`/
// `wrap_browser_process_handler!` macro blocks plus the small amount of
// macOS FFI (`ensure_macos_native_window_buttons`) they call directly.
// Pure/stateless helpers that those macro bodies call but that don't
// depend back on them live in sibling modules and are re-exported here
// so every existing `crate::app::*` call site (in and out of this
// module) keeps working unchanged:
//   - `gpu`             — Linux GPU-tier probing for ANGLE selection.
//   - `monitor`          — monitor work-area / DPI / centered-rect math.
//   - `window_settings`  — Linux window-properties FFI override,
//                           `SELECTED_OZONE_PLATFORM`, and the
//                           `window:transparent` settings.json reader.

use cef::*;
use std::cell::RefCell;
use std::sync::Arc;

use crate::client::*;
use crate::state::AppState;

mod gpu;
mod monitor;
mod window_settings;

#[cfg(target_os = "linux")]
pub(crate) use gpu::{detect_gpu_tier, GpuTier};
pub(crate) use monitor::get_monitor_centered_70pct;
pub use monitor::get_monitor_work_area;
#[cfg(target_os = "windows")]
pub use monitor::{dpi_scale_at, get_monitor_work_area_physical};
#[cfg(target_os = "linux")]
pub use window_settings::{install_linux_window_properties_override, SELECTED_OZONE_PLATFORM};
pub(crate) use window_settings::read_window_transparent_setting;

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

        // Windows only: floor the window at 380px wide so the tab strip and
        // widget bar can't be resized into an unusably cramped state. No
        // extra height floor beyond CEF/Windows' own default — only width
        // is constrained here.
        #[cfg(target_os = "windows")]
        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size {
                width: 380,
                height: 1,
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

            // Windows: bind (label, HWND) into state.window_hwnds right here,
            // at Views window-creation time — CefWindow::GetWindowHandle() is
            // reliable at this exact synchronous callback (BrowserHost::
            // window_handle(), used by capture_hwnd_for_label's fast path,
            // frequently returns null in CEF Views mode and can go null again
            // after the page loads — see window_pool.rs POOL_HWND_CACHE
            // comment for the diagnosed CEF behaviour).
            //
            // This closes the crash-reproject HWND cross-wire race
            // (docs/retro/retro-reproject-drag-hwnd-crosswire-2026-07-12.md):
            // capture_hwnd_for_label's EnumWindows fallback picks "whichever
            // of our own visible-but-unclaimed HWNDs comes first," which is
            // only safe if windows are created one at a time — reproject's
            // rapid back-to-back CreateWindowTask posts violate that, and two
            // windows' fallbacks can cross-wire which label owns which HWND
            // (a mis-binding that then latches permanently, since it passes
            // the cache's own IsWindow liveness check on every later read).
            // The CEF UI thread runs on_window_created for one window fully
            // to completion before the next queued CreateWindowTask begins,
            // so binding here — before either window's renderer has even
            // started loading, let alone signalled "ready" — is race-free by
            // construction. Mirrors the same pattern already proven for
            // floating panes (floating_pane.rs's CreateFloatingWindowTask
            // inserts into window_hwnds immediately after CreateWindowExW,
            // before the browser is embedded).
            //
            // capture_hwnd_for_label's own "already registered" guard means
            // it now becomes a no-op for every window that goes through this
            // path; its EnumWindows fallback remains only as a defensive
            // last resort for any exotic caller that doesn't carry a
            // window_registration.
            #[cfg(target_os = "windows")]
            if let Some((state, label)) = self.window_registration.as_ref() {
                let raw_hwnd = window.window_handle().0 as *mut std::ffi::c_void;
                if !raw_hwnd.is_null() {
                    state.window_hwnds.lock().insert(label.clone(), raw_hwnd as isize);
                }

                // Pool windows additionally need the taskbar-hiding /
                // promote-time bookkeeping below — unrelated to the
                // window_hwnds bind above, kept as its own branch.
                if label.starts_with("window-pool-") {
                    if !raw_hwnd.is_null() {
                        crate::commands::window_pool::init_pool_window_hwnd(label, raw_hwnd);
                    }
                    // Cache the CEF Views Window itself (valid here; lost post-load
                    // via browser_view.window()) so the promote can run the
                    // macOS-parity set_bounds()+show() visibility fix.
                    crate::commands::window_pool::cache_pool_window_view(label, window);
                }
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

                // macOS tab-redock (SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24):
                // cache this window's NSWindow.windowNumber → label here,
                // on the CEF UI thread, where reading it off the NSView is
                // safe — the CGEventTap hook thread only ever does a
                // read-only Mutex lookup against this cache, never touches
                // AppKit directly (see tear_off_hook.rs's macOS module doc
                // comment for why that distinction matters).
                #[cfg(target_os = "macos")]
                {
                    let nsview = window.window_handle() as *mut std::ffi::c_void;
                    if !nsview.is_null() {
                        if let Some(number) = unsafe { macos_window_number(nsview) } {
                            crate::commands::tear_off_hook::macos_register_window_number(
                                number,
                                label.clone(),
                            );
                        }
                    }
                }

                // Startup pool fill — Windows uses the launcher saga path
                // (saga_dispatch.rs::LiveActionRunner is cfg(windows) only).
                // Non-Windows has no separate launcher, so the host seeds the
                // pool here instead. Phase 7 (SPEC_POOL_PHASE7_MACOS_LINUX_2026_06_19.md)
                // implemented promote_pool_window for non-Windows and confirmed
                // that CEF Views set_bounds() correctly repositions from the
                // off-screen holding position (-32000,-32000) to the tear-off
                // destination. Wayland sessions run under XWayland (X11
                // backend forced), so the off-screen coords are invisible there
                // too. Both former blockers are resolved.
                //
                // NOTE: init_pool() is called from on_after_created("main") in
                // client/mod.rs:664 on ALL platforms — this block (on_window_created)
                // only runs the CEF Views window registration for non-Windows.
                // No duplicate pool init call needed here.
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
            // Close-cascade diagnostics (window-lifecycle-leak retro round 3,
            // 2026-07-05): trace window destruction so the close-debug file
            // shows whether the Window died with or without a browser
            // teardown having been initiated first.
            crate::client::dlog(&format!(
                "on_window_destroyed({})",
                self.window_registration
                    .as_ref()
                    .map(|(_, l)| l.as_str())
                    .unwrap_or("<unregistered>")
            ));
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

                // macOS tab-redock: drop this label from the windowNumber
                // cache too — a stale entry would let the CGEventTap hook
                // hit-test resolve a merge onto a window that no longer
                // exists.
                #[cfg(target_os = "macos")]
                crate::commands::tear_off_hook::macos_unregister_window_label(label);
            }
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            // Close-cascade diagnostics (window-lifecycle-leak retro round 3,
            // 2026-07-05): can_close + do_close + on_before_close are the
            // three links that decide whether a Views window close actually
            // destroys the hosted browser. None were observed firing for
            // secondary-window closes — instrument every outcome so the break
            // point is visible in the close-debug trace.
            let label_owned;
            let label = match self.window_registration.as_ref() {
                Some((_, l)) => l.as_str(),
                None => {
                    label_owned = String::from("<unregistered>");
                    label_owned.as_str()
                }
            };
            let browser_view = self.browser_view.borrow();
            let Some(browser_view) = browser_view.as_ref() else {
                crate::client::dlog(&format!(
                    "can_close({label}): browser_view=None -> allow close WITHOUT browser teardown"
                ));
                return 1;
            };
            if let Some(browser) = browser_view.browser() {
                match browser.host() {
                    Some(host) => {
                        let r = host.try_close_browser();
                        crate::client::dlog(&format!(
                            "can_close({label}): try_close_browser -> {r}"
                        ));
                        r
                    }
                    None => {
                        crate::client::dlog(&format!(
                            "can_close({label}): no host -> allow close"
                        ));
                        1 // no host yet (pre-init teardown) — allow close
                    }
                }
            } else {
                crate::client::dlog(&format!(
                    "can_close({label}): browser_view has no browser -> allow close"
                ));
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
            // OAuth sign-in popup (non-DevTools): give it an explicit tall,
            // centered window — a portrait-ish shape suits a sign-in form, and
            // it's ~2x the height of the default popup (per user request). Uses
            // the primary monitor's work area; falls back to the delegate's
            // default sizing when the work area can't be resolved (macOS/Linux
            // stubs). DevTools keeps its default sizing.
            let initial_bounds = if is_devtools == 0 {
                get_monitor_work_area(0, 0).map(|(wx, wy, ww, wh)| {
                    let w = (ww as f64 * 0.55).round() as i32;
                    let h = (wh as f64 * 0.95).round() as i32;
                    (wx + (ww - w) / 2, wy + (wh - h) / 2, w, h)
                })
            } else {
                None
            };
            let mut window_delegate = AgentMuxWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
                initial_bounds,
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

/// macOS tab-redock (SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24): read
/// `[[nsview window] windowNumber]`. Called only from `on_window_created`,
/// on the CEF UI thread — safe, one-time read whose result is cached in
/// `tear_off_hook`'s windowNumber→label map for the CGEventTap hook
/// thread to consult read-only (that thread must never touch AppKit
/// directly — see that module's doc comment). Returns `None` if the
/// NSWindow isn't resolvable (defensive; shouldn't happen for a just-
/// created top-level window).
#[cfg(target_os = "macos")]
unsafe fn macos_window_number(nsview: *mut std::ffi::c_void) -> Option<i64> {
    use std::ffi::c_void;
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> Sel;
        fn objc_msgSend();
    }
    let sel_window = sel_registerName(b"window\0".as_ptr() as _);
    let get_window: extern "C" fn(Id, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = get_window(nsview, sel_window);
    if nswindow.is_null() {
        return None;
    }
    let sel_window_number = sel_registerName(b"windowNumber\0".as_ptr() as _);
    // NSWindow.windowNumber is an NSInteger — i64 on 64-bit (the only
    // architectures this app targets).
    let get_window_number: extern "C" fn(Id, Sel) -> i64 =
        std::mem::transmute(objc_msgSend as *const c_void);
    Some(get_window_number(nswindow, sel_window_number))
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
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                // Proactive memory-pressure guard (SPEC_MEMORY_PRESSURE_
                // SUPERVISION_2026_06_16 §5.H): if the system is already
                // critically low on commit at launch, start in software
                // rendering. The GPU process is a large commit consumer, and on
                // a starved system spawning it can push commit over the edge and
                // OOM the host before first paint — the 2026-06-16 failure mode
                // under several concurrent instances. Browser process only (only
                // it decides whether to spawn the GPU process); reuses the same
                // `disable-gpu` switch as the launcher's degraded-relaunch rung.
                // On non-Windows `commit_free_mb()` is u64::MAX, so this never
                // fires there (commit-limit exhaustion is a Windows concern).
                const STARTUP_DISABLE_GPU_FLOOR_MB: u64 = 512;
                if process_type.is_none() {
                    let free = crate::memory_heartbeat::commit_free_mb();
                    if free < STARTUP_DISABLE_GPU_FLOOR_MB {
                        tracing::warn!(
                            target: "mem_pressure",
                            commit_free_mb = free,
                            floor_mb = STARTUP_DISABLE_GPU_FLOOR_MB,
                            "commit critically low at startup — launching with --disable-gpu (software rendering)"
                        );
                        cmd.append_switch(Some(&CefString::from("disable-gpu")));
                    }
                }

                // macOS dev mode: the dev binary runs unsigned from a flat
                // directory, not inside a signed .app bundle. macOS sandbox
                // policy SIGTRAP-kills subprocesses (GPU, Network/utility) that
                // call APIs gated on code-signing — exit_code=5 from both the
                // GPU helper and the network service utility process. The
                // installed app is properly signed and never hits this.
                //
                // --no-sandbox disables all sandbox restrictions so subprocesses
                // can init normally (Metal GPU, network stack) from the unsigned
                // binary. Hardware GPU compositing is preserved — no --disable-gpu.
                #[cfg(target_os = "macos")]
                if process_type.is_none() && std::env::var("AGENTMUX_DEV").is_ok() {
                    tracing::info!("macOS dev mode — disabling sandbox for unsigned binary (hardware GPU retained)");
                    cmd.append_switch(Some(&CefString::from("no-sandbox")));
                }

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
                //
                //   MacAppCodeSignClone — Chromium's "code-sign safe updates"
                //     feature: at startup it clonefile()s the whole .app into a
                //     temp dir on the boot volume so an in-place auto-update can
                //     swap the on-disk bundle without invalidating the running
                //     process's code signature. clonefile() is single-volume, so
                //     when the bundle runs from a *read-only DMG* (a different
                //     volume from the temp dir) the clone fails EXDEV and Chromium
                //     CHECK-aborts inside cef_initialize → SIGTRAP, before any
                //     window is created. AgentMux ships its own launcher/updater
                //     and sets --disable-component-update, so Chromium never
                //     performs an in-place swap of its own bundle → the clone
                //     buys nothing and only adds this failure mode (plus a >1 GB
                //     temp copy). Disabling it lets the packaged app launch from
                //     any volume, DMG included.
                //     NOTE on timing: unlike the MachPort policy below (read
                //     BEFORE FeatureList init — runtime flag too late, needs the
                //     source patch), this is a genuine base::Feature, evaluated
                //     AFTER FeatureList init, so the --disable-features switch
                //     *should* apply — same path as the working entries above.
                //     The clone runs early, though, so VERIFY by launching a
                //     fresh build from a DMG; if it proves too-late like MachPort,
                //     fall back to a code_sign_clone_manager source patch
                //     (cf. docs/cef-patches/ for the established pattern).
                let key = CefString::from("disable-features");
                // MachPortRendezvous{Validate,Enforce}PeerRequirements: belt-and-
                // suspenders flags kept for defence in depth. NOTE: the actual fix
                // for the macOS-26 renderer crash (-67030 / errSecCSReqFailed) is
                // the source patch in docs/cef-patches/ (GetPeerValidationPolicy →
                // kNoValidation) — the policy is read before FeatureList init so
                // this runtime flag cannot apply in time and is NOT the load-bearing
                // mechanism. Do not remove the patch thinking these flags cover it.
                //   WebRtcHideLocalIpsWithMdns — Chromium's network service
                //     binds an mDNS (multicast DNS) listener on UDP
                //     0.0.0.0:5353 to mint `.local` hostnames that hide local
                //     IPs from WebRTC peers. It binds this proactively at
                //     startup even with no active WebRTC (confirmed via
                //     `netstat -ano`: a Chromium network subprocess holds
                //     `UDP 0.0.0.0:5353`). That all-interfaces bind is what
                //     raises the Windows Defender Firewall "Windows Security
                //     Alert" on first run — labelled "CEF Bootstrap
                //     Application" because agentmux-cef.exe is CEF's bootstrap
                //     stub, whose PE version-info carries CEF's default
                //     description (build.rs stamps the AgentMux metadata onto
                //     the .dll, not the stub). MediaRouter (the other mDNS
                //     source) is already disabled above; disabling this one
                //     removes the last proactive 5353 bind, so no firewall
                //     prompt. Tradeoff: a browser-pane page using
                //     RTCPeerConnection would expose local IPs instead of
                //     `.local` names (pre-2019 Chrome default) — acceptable for
                //     a local single-user workbench; AgentMux's own UI uses no
                //     WebRTC, and voice input (getUserMedia audio) is
                //     unaffected. See feedback_no_os_notices_at_launch.
                let val = CefString::from(
                    "CalculateNativeWinOcclusion,MediaRouter,PreconnectToSearch,\
                     AutofillServerCommunication,MachPortRendezvousValidatePeerRequirements,\
                     MachPortRendezvousEnforcePeerRequirements,MacAppCodeSignClone,\
                     WebRtcHideLocalIpsWithMdns",
                );
                cmd.append_switch_with_value(Some(&key), Some(&val));

                // ── Linux: prefer native Wayland when the session offers it ──
                // History: CEF 146/Chromium 146 had broken native-Wayland Ozone
                // (`WaylandZwpLinuxDmabuf::OnTrancheFlags Not implemented` →
                // cc::Scheduler `DidNotProduceFrame` on ~89% of Mutter's
                // `BeginFrame` → rAF stalls past 600 ms, typing visibly hangs),
                // so we pinned `--ozone-platform=x11` (XWayland) as a workaround.
                //
                // CEF 148 fixed it. Verified on GNOME/Mutter (CEF 148): native-
                // Wayland init is clean — no `Not implemented` callbacks, no
                // `DidNotProduceFrame`/GPU fallback, and typing is smooth. So we
                // no longer force XWayland. When the session exposes
                // WAYLAND_DISPLAY we select native Wayland; otherwise we append
                // nothing and let Chromium use its X11 default. An explicit
                // `AGENTMUX_OZONE_PLATFORM=<wayland|x11>` still pins a specific
                // backend (regression testing / per-machine override).
                #[cfg(target_os = "linux")]
                {
                    let forced = std::env::var("AGENTMUX_OZONE_PLATFORM")
                        .ok()
                        .filter(|s| !s.is_empty());
                    let ozone = forced.or_else(|| {
                        let on_wayland = std::env::var("WAYLAND_DISPLAY")
                            .map(|s| !s.is_empty())
                            .unwrap_or(false);
                        if !on_wayland {
                            return None;
                        }
                        // Track 1 window transparency (uniform whole-window
                        // alpha, SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01) is
                        // delivered via the EWMH `_NET_WM_WINDOW_OPACITY` X11
                        // property — native Wayland has no equivalent protocol
                        // Chromium supports. Route transparent windows through
                        // XWayland (the universal default until CEF 148) so the
                        // property applies; opaque users keep native Wayland.
                        // An explicit AGENTMUX_OZONE_PLATFORM still wins above.
                        if read_window_transparent_setting() {
                            tracing::info!(
                                "window:transparent=true → ozone-platform=x11 (XWayland) for _NET_WM_WINDOW_OPACITY"
                            );
                            Some("x11".to_string())
                        } else {
                            Some("wayland".to_string())
                        }
                    });
                    if let Some(platform) = ozone {
                        let oz_key = CefString::from("ozone-platform");
                        let oz_val = CefString::from(platform.as_str());
                        cmd.append_switch_with_value(Some(&oz_key), Some(&oz_val));
                        let _ = crate::app::SELECTED_OZONE_PLATFORM.set(platform);
                    }

                    // Linux sandbox: use kernel namespace isolation instead of the setuid
                    // chrome-sandbox binary. On kernels ≥3.8 without user-namespace
                    // restrictions (standard distro default), Chromium's namespace
                    // sandbox is equivalent to the SUID path in practice.
                    // If the environment doesn't support user namespaces (older kernel,
                    // Docker with --security-opt=no-new-privileges, nested VM), set
                    // AGENTMUX_UNSAFE_NOSANDBOX=1 to fall back to no_sandbox=1.
                    #[cfg(feature = "sandbox")]
                    {
                        let suppress = std::env::var("AGENTMUX_UNSAFE_NOSANDBOX")
                            .map(|v| v.trim() == "1")
                            .unwrap_or(false);
                        if !suppress {
                            cmd.append_switch(Some(&CefString::from("disable-setuid-sandbox")));
                        }
                    }
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

                // Read window:transparent from settings.json before CefInitialize
                // so we can gate the transparent-compositing flags below.
                // Stored in AppState so on_context_initialized can pass it to
                // the frontend via the URL query string (early opacity hint).
                // Only runs in the browser process (process_type == None).
                let is_transparent = if process_type.is_none() {
                    let t = read_window_transparent_setting();
                    self.state.window_transparent.store(t, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!("window:transparent={}", t);
                    t
                } else {
                    false
                };

                if is_transparent {
                // Initial background color, ARGB hex. alpha=00 → fully
                // transparent → first-frame paint is alpha-aware so the
                // CSS body background's rgba() composes with the desktop
                // wallpaper. Only applied when window:transparent=true;
                // opaque windows keep the default (ff) background so
                // Chromium can use opaque compositing and LCD text.
                // Pair with: BrowserSettings.background_color = 0, and the
                // is_frameless main window delegate.
                let bg_key = CefString::from("background-color");
                let bg_val = CefString::from("00000000");
                cmd.append_switch_with_value(Some(&bg_key), Some(&bg_val));

                // Disable LCD text rendering — required for transparent
                // compositing. LCD subpixel AA requires opaque backgrounds;
                // Chromium force-sets contents_opaque=true on LCD-rendered
                // layers, defeating the alpha cascade. Grayscale AA on a
                // translucent UI is the standard tradeoff. Skipped for opaque
                // windows to preserve subpixel rendering quality.
                let lcd_key = CefString::from("disable-lcd-text");
                cmd.append_switch(Some(&lcd_key));
                } // end if is_transparent

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

                // Disable Chromium's Web Notifications API. We don't use
                // `new Notification()` anywhere; CEF still registers both the
                // main app and AgentMux Helper (Alerts) as OS notification
                // sources and requests macOS permission for each at startup,
                // showing two permission toasts on every new-version install.
                // Suppressing the API at the command-line level prevents both
                // registrations and eliminates the duplicate prompts.
                cmd.append_switch(Some(&CefString::from("disable-notifications")));

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
                // 2026-05-09. See docs/specs/archive/linux-cef-flags-audit-2026-05-08.md.
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
                    AgentMuxHandler::new(self.state.clone()),
                    false, // is_browser_pane = false — main browser takes focus normally
                ));
            }

            // Browser settings.
            let is_transparent = self.state.window_transparent.load(std::sync::atomic::Ordering::Relaxed);
            let settings = BrowserSettings {
                windowless_frame_rate: 60,
                // ARGB: alpha=0 → SK_AlphaTRANSPARENT → enables Views-framework
                // transparency. Only set when window:transparent=true; opaque
                // windows use 0xFF000000 so Chromium's compositor treats layers as
                // opaque (better performance, subpixel LCD text, no opacity flash).
                background_color: if is_transparent { 0x00000000 } else { 0xFF000000 },
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
                // Dev builds (`task dev`, with or without the launcher) load the
                // Vite dev server; packaged builds use the bundled frontend.
                // Identify by the host exe PATH (`is_dev_self`), not
                // `AGENTMUX_RUNTIME_MODE` — a parent dev AgentMux leaks that env
                // into descendants, which would wrongly flip a packaged build to
                // dev. (`has_frontend` below still wins, so this is also robust.)
                let is_dev = agentmux_common::is_dev_self();
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
                    // docs/analysis/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md.
                    let port: u16 = std::env::var("AGENTMUX_VITE_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(5173);
                    format!("http://localhost:{}", port)
                }
            } else {
                base_url
            };

            // Append IPC port, token, and transparency hint as URL query parameters
            // so the frontend can detect CEF mode and set the initial --window-opacity
            // CSS variable before first paint (avoiding the translucent-default flash
            // for window:transparent=false users).
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let url_with_ipc = format!(
                "{}{}ipc_port={}&ipc_token={}&window_transparent={}",
                base_url, separator, self.ipc_port, self.state.ipc_token,
                if is_transparent { "1" } else { "0" }
            );
            let url = CefString::from(url_with_ipc.as_str());

            tracing::info!("Loading URL: {}{}ipc_port={}&ipc_token=<redacted>&window_transparent={}", base_url, separator, self.ipc_port, if is_transparent { "1" } else { "0" });

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
