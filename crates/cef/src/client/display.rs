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
        let mut had_title = title.is_some();

        let mut browser = browser.cloned();
        // A popup window has no address bar, so its title leads with where it
        // is, which the page can't set (native-popups spec §8.4, N2).
        if let Some(b) = browser.as_ref().filter(|b| self.popup_browser_ids.contains(&b.identifier())) {
            self.popup_titles.insert(b.identifier(), title_str.clone());
            if let Some(t) = popup_title(b, &title_str) {
                display_title_str = t;
                had_title = true;
            }
        }
        set_window_title(&mut browser, had_title.then_some(display_title_str.as_str()));

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
            set_window_title(&mut Some(b.clone()), Some(&t));
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
