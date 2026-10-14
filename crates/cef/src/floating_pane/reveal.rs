// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keep a cold floating pane off the screen until its page has painted.
//!
//! A floating pane opened without a warm pool window creates its popup and
//! shows it at once, then embeds the browser. For the 35–90 ms before that
//! browser composites its first frame the visible window is white
//! (docs/reports/REPORT_FLOATING_PANE_OPEN_FLASH_RCA_2026_10_14.md §5). The
//! popup is now shown CLOAKED: Win32 and Chromium see an ordinary visible
//! window, so the page loads and paints exactly as before, but DWM doesn't
//! put it on the screen. The frontend's `report_first_paint` (a double rAF in
//! `bootstrap.ts`, sent with the window's own label) uncloaks it, by then
//! showing the static splash. A backstop uncloaks it anyway if that signal
//! never comes.
//!
//! Cloaking rather than hiding: a hidden window's page may never present a
//! frame, which would leave nothing to wait for.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// How long a cold floater stays cloaked when its page never reports a
/// painted frame. The report lands ~250 ms after the open on a loaded
/// machine; this is a backstop for a stalled renderer, not a target.
const REVEAL_BACKSTOP_MS: i64 = 2000;

/// Cloaked floaters waiting for their first paint: label → (HWND, cloaked at).
static PENDING: Mutex<Option<HashMap<String, (isize, Instant)>>> = Mutex::new(None);

fn set_cloak(hwnd: isize, on: bool) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
    let value: i32 = on as i32;
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd as *mut std::ffi::c_void,
            DWMWA_CLOAK as u32,
            &value as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    hr == 0
}

/// Cloak `hwnd` until `label`'s page paints. Call on the UI thread, before
/// the window is first shown.
pub(crate) fn cloak_until_painted(label: &str, hwnd: isize) {
    if !set_cloak(hwnd, true) {
        tracing::warn!(label = %label, "[floating-pane] DWMWA_CLOAK failed; the window shows before its first paint");
        return;
    }
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(label.to_string(), (hwnd, Instant::now()));
    let mut task = RevealTask::new(label.to_string(), "backstop");
    cef::post_delayed_task(cef::ThreadId::UI, Some(&mut task), REVEAL_BACKSTOP_MS);
}

/// `label`'s page reported a painted frame. Any thread.
pub(crate) fn on_first_paint(label: &str) {
    if !label.starts_with("floating-") {
        return;
    }
    let mut task = RevealTask::new(label.to_string(), "first-paint");
    cef::post_task(cef::ThreadId::UI, Some(&mut task));
}

/// Uncloak `label` if it is still waiting. Whichever of the paint signal and
/// the backstop comes first wins; the other finds nothing to do.
fn reveal(label: &str, reason: &str) {
    let entry = PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.remove(label));
    let Some((hwnd, cloaked_at)) = entry else { return };
    // Fails harmlessly if the window was closed meanwhile.
    set_cloak(hwnd, false);
    tracing::info!(
        target: "startup-paint",
        label = %label,
        reason,
        cloaked_ms = cloaked_at.elapsed().as_millis() as u64,
        "[floating-pane] cold floater revealed",
    );
}

mod task {
    use cef::*;

    wrap_task! {
        pub(super) struct RevealTask {
            label: String,
            reason: &'static str,
        }

        impl Task {
            fn execute(&self) {
                super::reveal(&self.label, self.reason);
            }
        }
    }
}
use task::RevealTask;
