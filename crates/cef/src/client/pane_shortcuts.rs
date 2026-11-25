// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The shortcuts a focused browser pane takes for itself (issue #1190): its keys go to CEF, not the app's
// dispatcher, so the host matches them in `on_pre_key_event` (handlers.rs) and runs them here.

/// Windows virtual-key codes for browser-pane-only shortcuts (issue #1190).
pub(super) const VK_L: i32 = 0x4C; // Ctrl+L — focus address bar
pub(super) const VK_R: i32 = 0x52; // Ctrl+R — reload
pub(super) const VK_N: i32 = 0x4E; // Ctrl+Shift+N — new Incognito tab
pub(super) const VK_HOME: i32 = 0x24; // Alt+Home — Home
pub(super) const VK_LEFT: i32 = 0x25; // Alt+Left — back
pub(super) const VK_RIGHT: i32 = 0x27; // Alt+Right — forward
pub(super) const VK_T: i32 = 0x54; // Ctrl+T — new tab in this pane
pub(super) const VK_W: i32 = 0x57; // Ctrl+W — close this tab
pub(super) const VK_F: i32 = 0x46; // Ctrl+F — find in the page

/// Reserved chrome-style shortcuts intercepted only when the focused CEF
/// browser is a browser pane (see issue #1190 — browser panes are native
/// CEF child windows, so normal app-level keydown handling never sees these
/// keystrokes). Kept independent of any CEF/AppState types so the matching
/// logic itself is unit-testable without a CEF runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserPaneShortcut {
    FocusAddress,
    Reload,
    GoBack,
    GoForward,
    /// Alt+Home, and Ctrl+Shift+N: the toolbar handles these
    /// (SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §2).
    Home,
    NewIncognito,
    /// Ctrl/Cmd+T: a new tab (a pane tab) beside this one; Ctrl/Cmd+W closes this one (never the pane's last).
    NewTab,
    CloseTab,
    /// Ctrl/Cmd+F: the find bar over the page (the app's own Find in pane has nothing to search here).
    Find,
}

/// `shift` must be false to match: codex P2 on #2548 — without this,
/// Ctrl+Shift+R (Chromium's hard-reload chord) was silently downgraded to a
/// plain cached reload, and Ctrl+Shift+L was hijacked as focus-address too.
/// Requiring shift-up leaves those shifted chords alone (falls through to
/// CEF's normal handling) instead of guessing what they should do.
pub(super) fn browser_pane_shortcut_for(ctrl: bool, alt: bool, shift: bool, vk: i32) -> Option<BrowserPaneShortcut> {
    // The one shifted chord taken: Ctrl+Shift+N, a new Incognito tab, as in a
    // browser. Every other shifted chord falls through to CEF, as below.
    if shift {
        return (ctrl && !alt && vk == VK_N).then_some(BrowserPaneShortcut::NewIncognito);
    }
    if ctrl && !alt {
        match vk {
            VK_L => Some(BrowserPaneShortcut::FocusAddress),
            VK_R => Some(BrowserPaneShortcut::Reload),
            VK_T => Some(BrowserPaneShortcut::NewTab),
            VK_W => Some(BrowserPaneShortcut::CloseTab),
            VK_F => Some(BrowserPaneShortcut::Find),
            _ => None,
        }
    } else if alt && !ctrl {
        match vk {
            VK_LEFT => Some(BrowserPaneShortcut::GoBack),
            VK_RIGHT => Some(BrowserPaneShortcut::GoForward),
            VK_HOME => Some(BrowserPaneShortcut::Home),
            _ => None,
        }
    } else {
        None
    }
}

/// The browser-pane shortcuts that win over an app shortcut on the same keys,
/// in a focused browser pane: Ctrl/Cmd+Shift+N opens an Incognito tab, as in a
/// browser, where the app has "new window"; Cmd+T and Cmd+W (macOS) open and close a
/// tab of the pane, where the app has "new window tab" and "close pane" (as the Files
/// pane does). Out of a pane they are still the app's (`run_browser_pane_shortcut`
/// declines there). Every other pane shortcut is on keys the app leaves free
/// (`pane_shortcuts_are_reachable`).
pub(super) fn pane_shortcut_before_app_keys(primary: bool, alt: bool, shift: bool, vk: i32) -> Option<BrowserPaneShortcut> {
    browser_pane_shortcut_for(primary, alt, shift, vk).filter(|s| {
        matches!(s, BrowserPaneShortcut::NewIncognito | BrowserPaneShortcut::NewTab | BrowserPaneShortcut::CloseTab | BrowserPaneShortcut::Find)
    })
}

#[cfg(test)]
mod browser_pane_shortcut_tests {
    use super::*;
    use crate::client::handlers::app_shortcut_for;

    const VK_P: i32 = 0x50; // Ctrl+P: the app's command palette, not a pane shortcut



    #[test]
    fn host_keys_json_parses_and_resolves_a_known_shortcut() {
        let _g = crate::client::host_keys::TEST_LOCK.lock();
        // Ctrl+Shift+T = new tab on Windows/Linux; ⌘T on macOS.
        assert_eq!(app_shortcut_for(true, true, false, false, 0x54, false).as_deref(), Some("tab:new"));
        assert_eq!(app_shortcut_for(false, false, false, true, 0x54, true).as_deref(), Some("tab:new"));
        assert_eq!(app_shortcut_for(true, false, false, false, 0x54, false), None);
    }

    /// Every browser-pane shortcut reaches the pane: either no app shortcut is
    /// on its keys, or it is taken before the app shortcuts. Ctrl/Cmd+Shift+N
    /// (and Alt+Home) were added without this list growing, so a pane's
    /// Ctrl+Shift+N opened a new window instead of an Incognito tab.
    #[test]
    fn pane_shortcuts_are_reachable() {
        let _g = crate::client::host_keys::TEST_LOCK.lock();
        let chords = [
            (true, false, false, VK_L),
            (true, false, false, VK_R),
            (false, true, false, VK_LEFT),
            (false, true, false, VK_RIGHT),
            (false, true, false, VK_HOME),
            (true, false, true, VK_N),
            (true, false, false, VK_T),
            (true, false, false, VK_W),
            (true, false, false, VK_F),
        ];
        for mac in [false, true] {
            for (primary, alt, shift, vk) in chords {
                let shortcut = browser_pane_shortcut_for(primary, alt, shift, vk);
                assert!(shortcut.is_some(), "mac={mac} vk={vk:#x}: not a pane shortcut");
                let (c, m) = if mac { (false, primary) } else { (primary, false) };
                if app_shortcut_for(c, shift, alt, m, vk, mac).is_some() {
                    assert_eq!(pane_shortcut_before_app_keys(primary, alt, shift, vk), shortcut, "mac={mac} vk={vk:#x}");
                }
            }
        }
    }

    #[test]
    fn a_panes_new_incognito_wins_over_new_window() {
        let _g = crate::client::host_keys::TEST_LOCK.lock();
        // The app's "new window" is on the same keys, on both platforms...
        assert_eq!(app_shortcut_for(true, true, false, false, VK_N, false).as_deref(), Some("window:new"));
        assert_eq!(app_shortcut_for(false, true, false, true, VK_N, true).as_deref(), Some("window:new"));
        // ...and a focused browser pane takes them first.
        assert_eq!(pane_shortcut_before_app_keys(true, false, true, VK_N), Some(BrowserPaneShortcut::NewIncognito));
        // No other pane shortcut is taken ahead of the app's.
        assert_eq!(pane_shortcut_before_app_keys(true, false, false, VK_L), None);
        assert_eq!(pane_shortcut_before_app_keys(false, true, false, VK_HOME), None);
    }

    #[test]
    fn ctrl_l_focuses_address_bar() {
        assert_eq!(
            browser_pane_shortcut_for(true, false, false, VK_L),
            Some(BrowserPaneShortcut::FocusAddress)
        );
    }

    #[test]
    fn ctrl_r_reloads() {
        assert_eq!(
            browser_pane_shortcut_for(true, false, false, VK_R),
            Some(BrowserPaneShortcut::Reload)
        );
    }

    #[test]
    fn alt_left_goes_back() {
        assert_eq!(
            browser_pane_shortcut_for(false, true, false, VK_LEFT),
            Some(BrowserPaneShortcut::GoBack)
        );
    }

    #[test]
    fn alt_right_goes_forward() {
        assert_eq!(
            browser_pane_shortcut_for(false, true, false, VK_RIGHT),
            Some(BrowserPaneShortcut::GoForward)
        );
    }

    #[test]
    fn plain_keypress_without_modifier_is_not_a_shortcut() {
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_LEFT), None);
    }

    #[test]
    fn ctrl_alt_combo_matches_neither_group() {
        // Ctrl+Alt+L / Ctrl+Alt+Left are not reserved shortcuts — only plain
        // Ctrl+<key> and plain Alt+<key> are matched, so a page/site binding
        // Ctrl+Alt+<key> for its own purposes isn't hijacked.
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_LEFT), None);
    }

    #[test]
    fn unrelated_key_is_not_a_shortcut() {
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_P), None);
    }

    #[test]
    fn ctrl_t_and_ctrl_w_open_and_close_a_tab() {
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_T), Some(BrowserPaneShortcut::NewTab));
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_W), Some(BrowserPaneShortcut::CloseTab));
        // A plain T or W, Alt or Shift with them, are the page's (Ctrl+Shift+T is the app's new window tab).
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_T), None);
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_W), None);
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_T), None);
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_W), None);
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_T), None);
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_W), None);
    }

    #[test]
    fn ctrl_f_finds_in_the_page() {
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_F), Some(BrowserPaneShortcut::Find));
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_F), None);
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_F), None);
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_F), None);
    }

    #[test]
    fn a_panes_tab_keys_win_over_the_apps_on_macos() {
        let _g = crate::client::host_keys::TEST_LOCK.lock();
        // The app's own Cmd+T is "new tab" and Cmd+W "close pane"...
        assert_eq!(app_shortcut_for(false, false, false, true, VK_T, true).as_deref(), Some("tab:new"));
        assert_eq!(app_shortcut_for(false, false, false, true, VK_W, true).as_deref(), Some("pane:close"));
        // ...and a focused browser pane takes them first, on every platform's primary key.
        assert_eq!(pane_shortcut_before_app_keys(true, false, false, VK_T), Some(BrowserPaneShortcut::NewTab));
        assert_eq!(pane_shortcut_before_app_keys(true, false, false, VK_W), Some(BrowserPaneShortcut::CloseTab));
        // Ctrl+F: a browser pane takes it for its page, whatever the app has on those keys.
        assert_eq!(pane_shortcut_before_app_keys(true, false, false, VK_F), Some(BrowserPaneShortcut::Find));
        // The pane's reload and address keys are still not taken ahead of the app's.
        assert_eq!(pane_shortcut_before_app_keys(true, false, false, VK_R), None);
    }

    #[test]
    fn shift_held_excludes_plain_shortcuts() {
        // codex P2 on #2548: Ctrl+Shift+R is Chromium's hard-reload chord and
        // Ctrl+Shift+L isn't a shortcut at all — neither should be silently
        // downgraded/hijacked into the plain Ctrl+R/Ctrl+L actions.
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_R), None);
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(false, true, true, VK_LEFT), None);
        assert_eq!(browser_pane_shortcut_for(false, true, true, VK_RIGHT), None);
    }
}
