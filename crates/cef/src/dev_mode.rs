// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dev mode on the host (docs/specs/SPEC_DEV_MODE_SETTING_2026_10_28.md §3.2).
//!
//! The frontend hides DevTools and Inspect Element while `app:devmode` is off.
//! The host checks the same value before it opens DevTools, so a stale window,
//! a script or a missed menu cannot open one while the setting says off. The
//! live `settings.json` is read each time (these commands are rare), so the
//! answer is always current and the host keeps no copy in step. The rule
//! itself, and its tests, are `agentmux_common::settings_jsonc::dev_mode_from`.

/// What a refused command answers with.
pub(crate) const OFF_MESSAGE: &str = "DevTools is off: turn on Dev mode in Settings › Advanced";

/// `app:devmode` if it is set, otherwise whether this is a task dev or debug build.
pub(crate) fn dev_mode_on() -> bool {
    let text = crate::sidecar::settings_file().and_then(|path| std::fs::read_to_string(path).ok());
    agentmux_common::settings_jsonc::dev_mode_from(text.as_deref(), agentmux_common::is_dev_self())
}

/// Ok while dev mode is on; the error a refused DevTools command returns otherwise.
pub(crate) fn require() -> Result<(), String> {
    if dev_mode_on() {
        Ok(())
    } else {
        Err(OFF_MESSAGE.to_string())
    }
}
