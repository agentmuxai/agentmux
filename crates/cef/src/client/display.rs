// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! DisplayHandler methods for `AgentMuxHandler` — title + favicon updates.
//! Extracted verbatim from client/mod.rs.

use cef::*;

use super::AgentMuxHandler;

/// `page_title` led by where `browser`'s main frame is
/// (`popup_rules::popup_window_title`), or `None` if it has nothing to show.
fn popup_title(browser: &Browser, page_title: &str) -> Option<String> {
    let url = browser.main_frame().map(|f| CefString::from(&ImplFrame::url(&f)).to_string())?;
    agentmux_common::popup_rules::popup_window_title(&url, page_title)
}

/// Set a popup window's title. Under the Chrome runtime CEF owns a popup's
/// window, so there is no Views window to set it on (lifecycle.rs `do_close`):
/// on Windows `set_window_title`'s Win32 path reaches it; on macOS and Linux
/// it is set here, on the top-level window that holds the popup's view.
/// Only for popups: a pane's view lives inside the AgentMux window, whose
/// title this would replace.
fn set_popup_window_title(browser: &mut Option<Browser>, title: &str) {
    set_window_title(browser, Some(title));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Some(host) = browser.as_ref().and_then(|b| b.host()) {
        #[cfg(target_os = "macos")]
        {
            let nsview = host.window_handle() as *mut std::ffi::c_void;
            if nsview.is_null() || !unsafe { macos_set_view_window_title(nsview, title) } {
                tracing::debug!("popup title: the popup's view has no window yet");
            }
        }
        #[cfg(target_os = "linux")]
        {
            let xid = host.window_handle() as u32;
            if xid != 0 {
                if let Err(e) = x11_set_toplevel_title(xid, title) {
                    tracing::debug!(error = %e, "popup title: couldn't set the X11 window title");
                }
            }
        }
    }
}

/// `[[nsview window] setTitle:title]`. CEF documents the handle as the
/// top-level native window for a browser it hosts, so it may be the window
/// itself: whichever it is, ask what it responds to before sending anything.
/// False when there's no window to title.
#[cfg(target_os = "macos")]
unsafe fn macos_set_view_window_title(handle: *mut std::ffi::c_void, title: &str) -> bool {
    use std::ffi::{c_char, c_void, CString};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    let Ok(title) = CString::new(title.replace('\0', "")) else { return false };
    let msg: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let responds: extern "C" fn(Id, Sel, Sel) -> bool = std::mem::transmute(objc_msgSend as *const c_void);
    let responds_to_sel = sel_registerName(b"respondsToSelector:\0".as_ptr() as _);
    let set_title_sel = sel_registerName(b"setTitle:\0".as_ptr() as _);
    let window_sel = sel_registerName(b"window\0".as_ptr() as _);
    let nswindow = if responds(handle, responds_to_sel, set_title_sel) {
        handle
    } else if responds(handle, responds_to_sel, window_sel) {
        msg(handle, window_sel)
    } else {
        std::ptr::null_mut()
    };
    if nswindow.is_null() || !responds(nswindow, responds_to_sel, set_title_sel) {
        return false;
    }
    let string_with: extern "C" fn(Id, Sel, *const c_char) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nsstring = string_with(
        objc_getClass(b"NSString\0".as_ptr() as _),
        sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _),
        title.as_ptr(),
    );
    if nsstring.is_null() {
        return false;
    }
    let set_title: extern "C" fn(Id, Sel, Id) = std::mem::transmute(objc_msgSend as *const c_void);
    set_title(nswindow, set_title_sel, nsstring);
    true
}

/// Set `_NET_WM_NAME` (and `WM_NAME`) on the client top-level window at or
/// above `xid`: the first with `WM_STATE`, which the window manager sets on
/// the windows it manages (not its own frame around them). `xid` itself if
/// none has it.
#[cfg(target_os = "linux")]
fn x11_set_toplevel_title(xid: u32, title: &str) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    let (conn, _screen) = x11rb::connect(None)?;
    let wm_state = conn.intern_atom(false, b"WM_STATE")?.reply()?.atom;
    let mut target = xid;
    let mut win = xid;
    for _ in 0..16 {
        let state = conn.get_property(false, win, wm_state, AtomEnum::ANY, 0, 0)?.reply()?;
        if state.type_ != x11rb::NONE {
            target = win;
            break;
        }
        let tree = conn.query_tree(win)?.reply()?;
        if tree.parent == tree.root || tree.parent == x11rb::NONE {
            break;
        }
        win = tree.parent;
    }
    let win = target;
    let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
    let utf8 = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
    conn.change_property8(PropMode::REPLACE, win, net_wm_name, utf8, title.as_bytes())?.check()?;
    conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, title.as_bytes())?
        .check()?;
    conn.flush()?;
    Ok(())
}

/// Set `browser`'s window title: through CEF Views, and for Alloy-style
/// native windows on Windows through Win32.
fn set_window_title(browser: &mut Option<Browser>, title: Option<&str>) {
    let owned = title.map(CefString::from);
    if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
        if let Some(window) = browser_view.window() {
            window.set_title(owned.as_ref());
        }
    }
    // Reagent P1 on #876: only call SetWindowTextW when CEF gave us an
    // actual title. CEF fires `on_title_change` with `title = None` in
    // several paths (e.g. about:blank, popup blockers) — passing "" to
    // SetWindowTextW would blank the application window title in those
    // cases. Preserve the existing title by skipping the Win32 update
    // when title is None.
    #[cfg(target_os = "windows")]
    if let (Some(title), Some(browser)) = (title, browser.as_ref()) {
        if let Some(host) = browser.host() {
            let hwnd = host.window_handle();
            if !hwnd.0.is_null() {
                let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
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

impl AgentMuxHandler {
    pub(crate) fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let title_str = title.map(|t| t.to_string()).unwrap_or_default();

        // Main-window-only display title. Appends the "running unsandboxed"
        // indicator (see linux_sandbox::RUNNING_UNSANDBOXED's doc comment)
        // ONLY when this is the main app window — never for Browser-widget
        // panes showing arbitrary web content. This handler fires for every
        // Browser instance, including user-opened Browser panes (reagent P1
        // on PR #2783: a Browser pane showing e.g. google.com would
        // otherwise report its title as "Google — Sandbox Disabled" via the
        // browser-pane-title-change event below for the rest of the
        // session). `title_str` itself stays exactly what the page
        // reported, unmodified, for that event.
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut display_title_str = title_str.clone();
        #[cfg(target_os = "linux")]
        if !self.is_browser_pane
            && crate::linux_sandbox::RUNNING_UNSANDBOXED.load(std::sync::atomic::Ordering::Relaxed)
        {
            display_title_str.push_str(" — Sandbox Disabled");
        }
        let had_title = title.is_some();

        let mut browser = browser.cloned();
        // A popup window has no address bar, so its title leads with where it
        // is, which the page can't set (native-popups spec §8.4, N2).
        let popup = browser.as_ref().filter(|b| self.popup_browser_ids.contains(&b.identifier())).and_then(|b| {
            self.popup_titles.insert(b.identifier(), title_str.clone());
            popup_title(b, &title_str)
        });
        match popup {
            Some(t) => set_popup_window_title(&mut browser, &t),
            None => set_window_title(&mut browser, had_title.then_some(display_title_str.as_str())),
        }

        // Emit live title to frontend for browser panes.
        if self.is_browser_pane {
            if let Some(b) = browser.as_ref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    let block_id_short: String = block_id.chars().take(7).collect();
                    tracing::info!(
                        "[browser-pane:diag][{}] emit-title-change title={:?}",
                        block_id_short,
                        title_str,
                    );
                    crate::events::emit_browser_pane_event(
                        &self.state,
                        &block_id,
                        "browser-pane-title-change",
                        &serde_json::json!({ "block_id": block_id, "title": title_str }),
                    );
                }
            }
        }
    }

    /// A popup window that navigates gets its new place in the title at once,
    /// rather than keeping the old site's until the new page sets a title.
    pub(crate) fn on_address_change(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        url: Option<&CefString>,
    ) {
        let (Some(b), Some(url)) = (browser, url) else { return };
        if !frame.is_some_and(|f| f.is_main() == 1) {
            return;
        }
        let Some(page_title) = self.popup_titles.get(&b.identifier()) else { return };
        if let Some(t) = agentmux_common::popup_rules::popup_window_title(&url.to_string(), page_title) {
            set_popup_window_title(&mut Some(b.clone()), &t);
        }
    }

    pub(crate) fn on_favicon_urlchange(
        &mut self,
        browser: Option<&mut Browser>,
        icon_urls: Option<&mut CefStringList>,
    ) {
        if !self.is_browser_pane {
            return;
        }
        let Some(b) = browser.as_deref() else { return };
        let Some(block_id) =
            crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
        else {
            return;
        };

        // Collect favicon URLs from the CefStringList. The list is an in-param
        // provided by CEF — we read via the raw sys API so we don't need to
        // consume (move) the borrowed reference.
        //
        // Reagent P1 on #876: `cef_string_list_value` writes into a
        // `cef_string_t` whose `str_` field points at a freshly-allocated
        // buffer owned by the list value (with `dtor` set to release it).
        // Dropping `value` as a plain Rust struct would leak that buffer on
        // every favicon URL CEF reports. After reading the string, we must
        // invoke the dtor manually to free the buffer.
        let urls: Vec<String> = if let Some(list) = icon_urls {
            let raw: *mut cef::sys::_cef_string_list_t = list.into();
            if let Some(raw_ref) = unsafe { raw.as_mut() } {
                let count = unsafe { cef::sys::cef_string_list_size(raw_ref) };
                (0..count)
                    .filter_map(|i| unsafe {
                        let mut value: cef::sys::cef_string_t = std::mem::zeroed();
                        if cef::sys::cef_string_list_value(raw_ref, i, &mut value) > 0 {
                            let s = CefString::from(std::ptr::from_ref(&value)).to_string();
                            // Free the buffer CEF allocated into `value.str_`.
                            if let Some(dtor) = value.dtor {
                                dtor(value.str_);
                            }
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Same list for the same page as last time: nothing to tell the page.
        let page_url = b
            .main_frame()
            .map(|f| CefString::from(&ImplFrame::url(&f)).to_string())
            .unwrap_or_default();
        let sent = (page_url, urls.clone());
        if self.sent_favicons.get(&b.identifier()) == Some(&sent) {
            return;
        }
        self.sent_favicons.insert(b.identifier(), sent);

        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-favicon-urls count={} first={:?}",
            block_id_short,
            urls.len(),
            urls.first(),
        );
        crate::events::emit_browser_pane_event(
            &self.state,
            &block_id,
            "browser-pane-favicon-urls",
            &serde_json::json!({ "block_id": block_id, "urls": urls }),
        );
    }
}
