// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — class-name filter for the WRR Win32 event hook.
//
// Hook callbacks fire for every window-object event in the host
// process: CEF browser top-levels (the ones we care about), CEF
// subprocess HWNDs, OS-managed transient HWNDs (tooltips, IME
// candidate lists, message-only windows), and so on. Forwarding
// every event over the IPC pipe would flood the wire with noise
// the reducer can't act on.
//
// `is_app_class` filters at the hook callback so only candidate
// AgentMux top-level windows produce IPC traffic. The classifier
// is allowlist-based — false negatives (a real window we silently
// drop) are worse than false positives (a non-app HWND that the
// reducer just ignores), so we keep the allowlist conservative
// and let the reducer's `pending_hwnds` machinery age out spurious
// entries.

/// Phase B.9.1 — does this Win32 window class look like an
/// AgentMux top-level window?
///
/// CEF's top-level windows in CEF 146 use the
/// `Chrome_WidgetWin_*` class family (CEF Views wraps each native
/// window in a Chromium widget host). AgentMux's main window
/// shows up as `Chrome_WidgetWin_1`. Browser-pane child HWNDs and
/// CEF subprocess HWNDs use `Chrome_RenderWidgetHostHWND` and
/// related — those are explicitly excluded.
pub fn is_app_class(class_name: &str) -> bool {
    // Chromium widget host windows. CEF wraps every top-level
    // CefWindow in one. The numeric suffix varies (`_0` `_1` etc.)
    // depending on CEF init order.
    if class_name.starts_with("Chrome_WidgetWin_") {
        return true;
    }
    // CEF Views Native Window (rare in our build but possible).
    if class_name == "CefBrowserWindow" {
        return true;
    }
    false
}

/// Phase B.9.1 — explicit excludes for class names that LOOK
/// app-like but should never produce drift. Currently empty
/// because `is_app_class` is allowlist-based; kept as a hook for
/// follow-up tuning if the allowlist gets too permissive.
pub fn is_explicitly_excluded(_class_name: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_widget_win_classes_match() {
        assert!(is_app_class("Chrome_WidgetWin_0"));
        assert!(is_app_class("Chrome_WidgetWin_1"));
        assert!(is_app_class("Chrome_WidgetWin_42"));
    }

    #[test]
    fn cef_browser_window_matches() {
        assert!(is_app_class("CefBrowserWindow"));
    }

    #[test]
    fn renderer_subprocess_class_is_filtered_out() {
        assert!(!is_app_class("Chrome_RenderWidgetHostHWND"));
        assert!(!is_app_class("Intermediate D3D Window"));
    }

    #[test]
    fn unrelated_classes_filtered_out() {
        assert!(!is_app_class("tooltips_class32"));
        assert!(!is_app_class("Static"));
        assert!(!is_app_class(""));
    }
}
