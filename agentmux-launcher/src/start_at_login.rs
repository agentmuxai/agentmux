// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app:startatlogin`: one setting, three views
//! (`docs/specs/SPEC_START_WITH_OS_2026_09_25.md` §3.9).
//!
//! The setting is the single source of truth, owned by srv like every other
//! setting. The Settings toggle writes it with `setconfig`; the tray's check
//! item writes it the same way, over the launcher's srv connection
//! (`notify::request_start_at_login`, the path the tray's "pause
//! notifications" already uses). srv broadcasts every change to all clients,
//! so the Settings page, the tray check mark and the OS login entry all follow
//! the one value: this module receives it (`observe`), refreshes the tray, and
//! reconciles the login entry to it (`autostart::plan`).
//!
//! Off unless the user turns it on (owner decision, 2026-09-25).

use std::sync::Mutex;

/// Settings key.
pub const SETTINGS_KEY: &str = "app:startatlogin";

/// The last value srv reported. `None` until the first config frame arrives:
/// the tray shows the item disabled until then, rather than guess.
static SEEN: Mutex<Option<Option<bool>>> = Mutex::new(None);

/// Serializes reconciles, so two quick toggles cannot interleave their writes.
static RECONCILE: Mutex<()> = Mutex::new(());

/// The setting as the tray should show it: `None` = not known yet.
pub fn current() -> Option<bool> {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).map(|v| v.unwrap_or(false))
}

/// srv reported the setting (on connect, and after every settings change).
/// `None` means the key is not set.
pub fn observe(setting: Option<bool>) {
    {
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        // Every settings change broadcasts the whole config; only act on this key.
        if *seen == Some(setting) {
            return;
        }
        *seen = Some(setting);
    }
    crate::tray::notify_menu::wake();
    // Blocking I/O (schtasks on Windows), so off the async session.
    std::thread::Builder::new()
        .name("agentmux-start-at-login".into())
        .spawn(reconcile_latest)
        .ok();
}

/// The tray's check item was clicked.
pub fn toggle() {
    if let Some(on) = current() {
        crate::notify::request_start_at_login(!on);
    }
    // Put the item back to the known value until srv confirms the change:
    // some menus flip their own check mark on click.
    crate::tray::notify_menu::wake();
}

fn reconcile_latest() {
    let _guard = RECONCILE.lock().unwrap_or_else(|e| e.into_inner());
    // Apply the newest value, not the one that spawned this thread, so a
    // burst of changes ends in the last state.
    let Some(setting) = *SEEN.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    let channel = crate::autostart::current_channel();
    // Only a write needs the target. Resolving it must not stand in the way of
    // turning start-at-login off (ReAgent P1 on #3788).
    let target = crate::autostart::stable_target();
    let target_str = target.as_ref().ok().map(|t| t.display().to_string());
    let entry = crate::autostart::read_entry();
    let plan = crate::autostart::plan(setting, entry.as_ref(), &channel, target_str.as_deref());
    let result = match plan {
        crate::autostart::Plan::Nothing => return,
        crate::autostart::Plan::Write => match &target {
            Ok(t) => crate::autostart::enable(t, &channel),
            Err(e) => Err(format!("no target to register: {e}")),
        },
        crate::autostart::Plan::Remove => crate::autostart::disable(),
        crate::autostart::Plan::Adopt => {
            crate::notify::request_start_at_login(true);
            Ok(())
        }
    };
    match result {
        Ok(()) => crate::log(&format!(
            "start-at-login: {plan:?} (setting={setting:?}, channel={channel}, target={target_str:?}, was={entry:?})"
        )),
        Err(e) => crate::log(&format!("start-at-login: {plan:?} failed: {e}")),
    }
}
