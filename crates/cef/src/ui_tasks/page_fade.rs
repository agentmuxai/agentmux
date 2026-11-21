// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window opacity carried by the page, on every platform (#48092).
//!
//! In a process that started alpha-capable (app/mod.rs `alpha_capable`; app/window_settings.rs `page_carries_fade`) this
//! is the only thing that applies the opacity: the page fades its own app root (theme.scss `am-uniform-fade`), and what
//! floats outside that root (a menu) is painted opaque and stays so over the desktop. Native Wayland already works this
//! way (`SetWindowAlphaTask`). A process that started opaque, to keep sub-pixel text smoothing, never gets here.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

#[cfg(target_os = "macos")]
use super::get_window_on_ui;
#[cfg(target_os = "macos")]
use super::window::macos_fade_pane_overlays;

wrap_task! {
    pub struct PageFadeTask {
        state: Arc<AppState>,
        label: String,
        alpha: f64,
    }

    impl Task {
        fn execute(&self) {
            // macOS: the host window must be see-through where the page is, and the browser-pane overlays (separate
            // windows the page cannot fade) are faded to the same alpha, as the OS fade does.
            #[cfg(target_os = "macos")]
            {
                let nsview = get_window_on_ui(&self.state, &self.label).map(|w| w.window_handle() as *mut std::ffi::c_void);
                match nsview {
                    Some(v) if !v.is_null() => {
                        if unsafe { macos_make_window_clear(v) } {
                            tracing::info!(label = %self.label, "[opacity] host NSWindow made non-opaque with a clear background");
                        } else {
                            tracing::warn!(label = %self.label, "[opacity] PageFadeTask: NSView has no NSWindow yet");
                        }
                    }
                    _ => tracing::warn!(label = %self.label, "[opacity] PageFadeTask: no window for label"),
                }
                macos_fade_pane_overlays(&self.state, &self.label, self.alpha);
                // The renderer's own base background is opaque (CefBrowserSettings.background_color is not honoured
                // for a windowed browser), so even a clear NSWindow shows black behind a faded page. Making it
                // transparent through DevTools is what lets the backdrop show through the faded panes.
                if let Some(browser) = self.state.get_browser(&self.label) {
                    keep_renderer_background(&browser, &self.label, self.alpha);
                }
            }
            let Some(frame) = self.state.get_browser(&self.label).and_then(|b| b.main_frame()) else {
                tracing::warn!(label = %self.label, "[opacity] PageFadeTask: no browser for label");
                return;
            };
            let code = CefString::from(uniform_fade_js(self.alpha).as_str());
            frame.execute_java_script(Some(&code), Some(&CefString::from("")), 0);
            tracing::info!(label = %self.label, alpha = self.alpha, "[opacity] applied page fade");
        }
    }
}

pub fn post_page_fade(state: &Arc<AppState>, label: &str, alpha: f64) {
    let mut task = PageFadeTask::new(state.clone(), label.to_string(), alpha);
    post_task(ThreadId::UI, Some(&mut task));
}

/// The DevTools message that makes the renderer's default background transparent while the page carries a fade, and
/// gives it back at full opacity. Its id is far above the ids `browser_api::cdp` hands out, and the reply is ignored.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn renderer_background_override(alpha: f64) -> String {
    const ID: i32 = 2_000_000_000;
    if alpha >= 1.0 {
        format!(r#"{{"id":{ID},"method":"Emulation.setDefaultBackgroundColorOverride"}}"#)
    } else {
        format!(
            r#"{{"id":{ID},"method":"Emulation.setDefaultBackgroundColorOverride","params":{{"color":{{"r":0,"g":0,"b":0,"a":0}}}}}}"#
        )
    }
}

#[cfg(target_os = "macos")]
thread_local! {
    /// The fade each browser carries, by browser id, and the observer that watches its DevTools sessions (UI thread
    /// only; dropping a `Registration` removes the observer).
    static FADES: std::cell::RefCell<std::collections::HashMap<i32, f64>> = Default::default();
    static WATCHERS: std::cell::RefCell<std::collections::HashMap<i32, Registration>> = Default::default();
}

#[cfg(target_os = "macos")]
wrap_dev_tools_message_observer! {
    struct BackgroundWatcher {}

    impl DevToolsMessageObserver {
        // Chromium drops a renderer background override together with the DevTools session that set it, so another
        // client (an agent's Browser tool, DevTools, a test driver) attaching and detaching turned the faded panes
        // opaque again until the next opacity change. Send it again when a session ends.
        fn on_dev_tools_agent_detached(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return };
            let alpha = FADES.with(|fades| fades.borrow().get(&browser.identifier()).copied());
            if let (Some(alpha), Some(host)) = (alpha, browser.host()) {
                let message = renderer_background_override(alpha);
                host.send_dev_tools_message(Some(message.as_bytes()));
            }
        }
    }
}

/// Make the renderer's default background transparent for a fade below 1.0 and keep it so, or give it back at 1.0.
#[cfg(target_os = "macos")]
fn keep_renderer_background(browser: &Browser, label: &str, alpha: f64) {
    let Some(host) = browser.host() else { return };
    if host.send_dev_tools_message(Some(renderer_background_override(alpha).as_bytes())) == 0 {
        tracing::warn!(label = %label, "[opacity] renderer background override was refused");
    }
    let id = browser.identifier();
    if alpha >= 1.0 {
        FADES.with(|fades| fades.borrow_mut().remove(&id));
        WATCHERS.with(|watchers| watchers.borrow_mut().remove(&id));
        return;
    }
    FADES.with(|fades| fades.borrow_mut().insert(id, alpha));
    WATCHERS.with(|watchers| {
        let mut watchers = watchers.borrow_mut();
        if !watchers.contains_key(&id) {
            let mut observer = BackgroundWatcher::new();
            if let Some(registration) = host.add_dev_tools_message_observer(Some(&mut observer)) {
                watchers.insert(id, registration);
            }
        }
    });
}

/// Script that fades the whole page to `alpha` (or clears the fade at 1.0).
pub(super) fn uniform_fade_js(alpha: f64) -> String {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha >= 1.0 {
        "(function(){var r=document.documentElement;r.classList.remove('am-uniform-fade');\
         r.style.removeProperty('--am-window-fade');})()"
            .to_string()
    } else {
        format!(
            "(function(){{var r=document.documentElement;r.classList.add('am-uniform-fade');\
             r.style.setProperty('--am-window-fade','{alpha}');}})()"
        )
    }
}

/// Make the host NSWindow see-through where the page is: non-opaque with a clear background, so the page's own
/// transparent pixels (and the faded app root) show the desktop. A stock NSWindow is opaque and paints its own
/// backing under a transparent page, so a page fade only darkens toward black. Done once per window: setting it again
/// makes the window server recomposite the whole window (it flickered a menu open in `pane_hole_mask.rs`). Also
/// re-casts the shadow, which follows the window's pixels. UI/main thread only. Returns false when the NSView has no
/// NSWindow yet.
#[cfg(target_os = "macos")]
unsafe fn macos_make_window_clear(nsview: *mut std::ffi::c_void) -> bool {
    use std::ffi::{c_char, c_void};
    use std::sync::Mutex;
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    static PREPARED: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

    let get_id: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let set_id: extern "C" fn(Id, Sel, Id) = std::mem::transmute(objc_msgSend as *const c_void);
    let set_bool: extern "C" fn(Id, Sel, u8) = std::mem::transmute(objc_msgSend as *const c_void);
    let call_void: extern "C" fn(Id, Sel) = std::mem::transmute(objc_msgSend as *const c_void);

    let nswindow = get_id(nsview, sel_registerName(b"window\0".as_ptr() as _));
    if nswindow.is_null() {
        return false;
    }
    let mut guard = PREPARED.lock().unwrap_or_else(|p| p.into_inner());
    if guard.get_or_insert_with(Default::default).insert(nswindow as usize) {
        set_bool(nswindow, sel_registerName(b"setOpaque:\0".as_ptr() as _), 0);
        let clear = get_id(objc_getClass(b"NSColor\0".as_ptr() as _), sel_registerName(b"clearColor\0".as_ptr() as _));
        set_id(nswindow, sel_registerName(b"setBackgroundColor:\0".as_ptr() as _), clear);
    }
    call_void(nswindow, sel_registerName(b"invalidateShadow\0".as_ptr() as _));
    true
}

#[cfg(test)]
mod tests {
    use super::{renderer_background_override, uniform_fade_js};

    #[test]
    fn renderer_background_override_is_transparent_under_a_fade_and_cleared_at_full() {
        let on: serde_json::Value = serde_json::from_str(&renderer_background_override(0.35)).unwrap();
        assert_eq!(on["method"], "Emulation.setDefaultBackgroundColorOverride");
        assert_eq!(on["params"]["color"]["a"], 0);
        let off: serde_json::Value = serde_json::from_str(&renderer_background_override(1.0)).unwrap();
        assert_eq!(off["method"], "Emulation.setDefaultBackgroundColorOverride");
        assert!(off.get("params").is_none());
    }

    #[test]
    fn uniform_fade_js_sets_and_clears_the_fade() {
        let on = uniform_fade_js(0.6);
        assert!(on.contains("classList.add('am-uniform-fade')"));
        assert!(on.contains("setProperty('--am-window-fade','0.6')"));

        for alpha in [1.0, 1.5] {
            let off = uniform_fade_js(alpha);
            assert!(off.contains("classList.remove('am-uniform-fade')"));
            assert!(off.contains("removeProperty('--am-window-fade')"));
        }

        assert!(uniform_fade_js(-1.0).contains("'--am-window-fade','0'"));
    }
}
