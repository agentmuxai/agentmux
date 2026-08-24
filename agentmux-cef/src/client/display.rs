// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! DisplayHandler methods for `AgentMuxHandler` — title + favicon updates.
//! Extracted verbatim from client/mod.rs.

use cef::*;

use super::AgentMuxHandler;

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
        let owned_title = CefString::from(display_title_str.as_str());
        let title: Option<&CefString> = if had_title { Some(&owned_title) } else { None };

        // Update the window title via CEF Views.
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        }
        // For Alloy-style native windows on Windows, update via Win32 API.
        // Reagent P1 on #876: only call SetWindowTextW when CEF gave us an
        // actual title. CEF fires `on_title_change` with `title = None` in
        // several paths (e.g. about:blank, popup blockers) — passing "" to
        // SetWindowTextW would blank the application window title in those
        // cases. Preserve the existing title by skipping the Win32 update
        // when title is None.
        #[cfg(target_os = "windows")]
        if title.is_some() {
            if let Some(browser) = browser.as_ref() {
                if let Some(host) = browser.host() {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        let title_wide: Vec<u16> = display_title_str
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
                    crate::events::emit_event_from_state(
                        &self.state,
                        "browser-pane-title-change",
                        &serde_json::json!({ "block_id": block_id, "title": title_str }),
                    );
                }
            }
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

        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-favicon-urls count={} first={:?}",
            block_id_short,
            urls.len(),
            urls.first(),
        );
        crate::events::emit_event_from_state(
            &self.state,
            "browser-pane-favicon-urls",
            &serde_json::json!({ "block_id": block_id, "urls": urls }),
        );
    }
}
