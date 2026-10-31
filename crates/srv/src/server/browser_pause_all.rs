// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Pause all agent browsing": the person's one emergency switch
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §7, D9). While
//! it is on, every pane an agent drives refuses every agent call, and the
//! clocks of their Auto-approve grants stand still (`browser_actions::is_paused`
//! answers for it, so everything that honours a pane's Pause honours this).
//!
//! It is kept in a file in the config folder, so a restart does not switch it
//! off: an emergency stop that reset itself would defeat its point. A file
//! that exists but can't be read counts as on. Only the user's own window
//! turns it on or off (`browsergrants.pauseall`, an operator-only command).

use std::sync::{Mutex, OnceLock};

use crate::backend::rpc_types::BrowserPauseAll;
use crate::server::AppState;

const FILE_NAME: &str = "browser-pause-all.json";

fn path() -> std::path::PathBuf {
    crate::backend::base::get_mux_config_dir().join(FILE_NAME)
}

/// Epoch milliseconds the switch was turned on at, if it is on. A file that
/// exists but can't be read, or has nothing readable in it, reads as on since
/// 0: fail closed. No file is off.
fn read_file(path: &std::path::Path) -> Option<i64> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(0),
        Ok(bytes) => Some(
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("since_ms").and_then(|m| m.as_i64()))
                .unwrap_or(0),
        ),
    }
}

fn slot() -> std::sync::MutexGuard<'static, Option<i64>> {
    static S: OnceLock<Mutex<Option<i64>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(read_file(&path()))).lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn is_on() -> bool {
    #[cfg(test)]
    if let Some(on) = FORCED.with(|f| f.get()) {
        return on;
    }
    slot().is_some()
}

#[cfg(test)]
thread_local! {
    /// A test's own answer for [`is_on`], on its own thread: the switch is
    /// process-wide and kept in the config folder, which a test must not touch.
    static FORCED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Make [`is_on`] answer `on` on this thread (`None`: the real switch again).
#[cfg(test)]
pub(crate) fn force_for_test(on: Option<bool>) {
    FORCED.with(|f| f.set(on));
}

/// For `Browser` `open`, which resolves no pane of its own: the caller's pane,
/// once its identity is verified, and a refusal while the switch is on (a new
/// pane is a pane an agent would drive).
pub(crate) fn open_block_id(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    auth: &agentmux_common::api_types::UiAutomationAuth,
) -> Result<String, (axum::http::StatusCode, String)> {
    let own = crate::server::ui_handlers::verified_block_id(state, caller, auth).map_err(|e| (axum::http::StatusCode::UNAUTHORIZED, e))?;
    if is_on() {
        let why = "the user paused all agent browsing; no pane was opened. Try again later, or ask them in chat";
        return Err((axum::http::StatusCode::CONFLICT, why.to_string()));
    }
    Ok(own)
}

pub(crate) fn status() -> BrowserPauseAll {
    let since = *slot();
    BrowserPauseAll { paused: since.is_some(), since_ms: since }
}

/// Turn the switch on or off. Returns where it stands now. A change is kept in
/// the file, held on every grant's clock, shown on every driven pane's header
/// and Log, and sent to the windows' status bars.
pub(crate) fn set(state: &AppState, on: bool) -> BrowserPauseAll {
    let changed = {
        let mut s = slot();
        match (on, *s) {
            (true, None) => {
                let now = chrono::Utc::now().timestamp_millis();
                *s = Some(now);
                save(&path(), Some(now));
                true
            }
            (false, Some(_)) => {
                *s = None;
                save(&path(), None);
                true
            }
            _ => false,
        }
    };
    let now = status();
    if changed {
        tracing::info!(paused = on, "[browser-pause-all] the user {} all agent browsing", if on { "paused" } else { "resumed" });
        crate::server::browser_actions::pause_all_changed(state, on);
        state.broker.publish_event::<crate::backend::mps_events::BrowserPauseAllChanged>(&now, vec![], 1);
    }
    now
}

fn save(path: &std::path::Path, since: Option<i64>) {
    let done = match since {
        Some(ms) => std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))
            .and_then(|()| std::fs::write(path, format!("{{\"since_ms\":{ms}}}"))),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        },
    };
    if let Err(e) = done {
        // The switch holds for this run; it won't survive a restart.
        tracing::warn!(error = %e, "[browser-pause-all] couldn't keep the switch in its file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_reads_as_on_and_a_missing_one_as_off() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        assert_eq!(read_file(&p), None);
        save(&p, Some(1234));
        assert_eq!(read_file(&p), Some(1234));
        std::fs::write(&p, "not json").unwrap();
        assert_eq!(read_file(&p), Some(0));
        save(&p, None);
        assert_eq!(read_file(&p), None);
        // Turning it off twice is fine.
        save(&p, None);
    }
}
