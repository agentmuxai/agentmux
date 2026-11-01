// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Pause all agent browsing": one emergency switch
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §7, D9). While
//! it is on, every pane an agent drives refuses every agent call, and the
//! clocks of their Auto-approve grants stand still (`browser_actions::is_paused`
//! answers for it, so everything that honours a pane's Pause honours this).
//!
//! It is kept in a file in the config folder, so a restart does not switch it
//! off: an emergency stop that reset itself would defeat its point. A file
//! that exists but can't be read counts as on.
//!
//! Anyone may turn it on (`browsergrants.pauseall`): the person from their
//! window, an agent from its connection, and it records who. The person turns
//! it off from their window. An agent asks for that as the sensitive operation
//! `resume_agent_browsing`: the super agent reviews it, the person sees a
//! countdown and can veto it, and it is recorded as that agent, never as the
//! person (docs/specs/SPEC_SUPER_AGENT_2026_10_25.md).

use std::sync::{Mutex, OnceLock};

use crate::backend::rpc_types::BrowserPauseAll;
use crate::server::AppState;

const FILE_NAME: &str = "browser-pause-all.json";

fn path() -> std::path::PathBuf {
    crate::backend::base::get_mux_config_dir().join(FILE_NAME)
}

/// The switch while it is on: when, and who turned it on (None: the person).
#[derive(Clone, Debug, PartialEq)]
struct Pause {
    since_ms: i64,
    by: Option<String>,
}

/// The switch as `path` holds it: on if the file is there. A file that exists but can't be read,
/// or has nothing readable in it, reads as on since 0: fail closed. No file is off.
fn read_file(path: &std::path::Path) -> Option<Pause> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Pause { since_ms: 0, by: None }),
        Ok(bytes) => {
            let v = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
            let field = |k: &str| v.as_ref().and_then(|v| v.get(k).cloned());
            Some(Pause {
                since_ms: field("since_ms").and_then(|m| m.as_i64()).unwrap_or(0),
                by: field("by").and_then(|b| b.as_str().map(str::to_string)).filter(|b| !b.is_empty()),
            })
        }
    }
}

fn slot() -> std::sync::MutexGuard<'static, Option<Pause>> {
    static S: OnceLock<Mutex<Option<Pause>>> = OnceLock::new();
    // A test never reads the real config folder: it starts with the switch off.
    S.get_or_init(|| Mutex::new(if cfg!(test) { None } else { read_file(&path()) })).lock().unwrap_or_else(|p| p.into_inner())
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
    let pause = slot().clone();
    BrowserPauseAll { paused: pause.is_some(), since_ms: pause.as_ref().map(|p| p.since_ms), by: pause.and_then(|p| p.by) }
}

/// Move `cur` to on or off. Returns whether that changed anything: turning it on again keeps who
/// turned it on first, and off again is nothing.
fn apply(cur: &mut Option<Pause>, on: bool, by: Option<&str>, now_ms: i64) -> bool {
    match (on, cur.is_some()) {
        (true, false) => {
            *cur = Some(Pause { since_ms: now_ms, by: by.map(str::to_string) });
            true
        }
        (false, true) => {
            *cur = None;
            true
        }
        _ => false,
    }
}

/// Turn the switch on or off, as `by` (None: the person). Returns where it stands now. A change
/// is kept in the file, held on every grant's clock, shown on every driven pane's header and Log
/// (as `by`, never as the person when an agent did it), and sent to the windows' status bars.
pub(crate) fn set(state: &AppState, on: bool, by: Option<&str>) -> BrowserPauseAll {
    // The file is written under the same hold as the slot, so two changes at once can't leave
    // the file saying one thing and memory another (a missing file means unpaused after a restart).
    let changed = {
        let mut s = slot();
        let changed = apply(&mut s, on, by, chrono::Utc::now().timestamp_millis());
        if changed {
            persist(s.as_ref());
        }
        changed
    };
    let now = status();
    if changed {
        tracing::info!(paused = on, by = by.unwrap_or("the user"), "[browser-pause-all] all agent browsing was {}", if on { "paused" } else { "resumed" });
        crate::server::browser_actions::pause_all_changed(state, on, by);
        state.broker.publish_event::<crate::backend::mps_events::BrowserPauseAllChanged>(&now, vec![], 1);
    }
    now
}

/// Keep the switch in its file. A test keeps nothing on disk.
fn persist(pause: Option<&Pause>) {
    if !cfg!(test) {
        save(&path(), pause);
    }
}

fn save(path: &std::path::Path, pause: Option<&Pause>) {
    let done = match pause {
        Some(p) => std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))
            .and_then(|()| std::fs::write(path, serde_json::json!({ "since_ms": p.since_ms, "by": p.by }).to_string())),
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
        save(&p, Some(&Pause { since_ms: 1234, by: None }));
        assert_eq!(read_file(&p), Some(Pause { since_ms: 1234, by: None }));
        std::fs::write(&p, "not json").unwrap();
        assert_eq!(read_file(&p), Some(Pause { since_ms: 0, by: None }));
        save(&p, None);
        assert_eq!(read_file(&p), None);
        // Turning it off twice is fine.
        save(&p, None);
        // A path that is there but can't be read as a file (here a folder) is on, not off.
        std::fs::create_dir(&p).unwrap();
        assert_eq!(read_file(&p), Some(Pause { since_ms: 0, by: None }));
    }

    #[test]
    fn the_file_keeps_who_paused_so_a_restart_still_says() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        save(&p, Some(&Pause { since_ms: 5, by: Some("naki-2".to_string()) }));
        assert_eq!(read_file(&p), Some(Pause { since_ms: 5, by: Some("naki-2".to_string()) }));
        // A file from before the switch recorded who reads as the person's.
        std::fs::write(&p, r#"{"since_ms":9}"#).unwrap();
        assert_eq!(read_file(&p), Some(Pause { since_ms: 9, by: None }));
    }

    #[test]
    fn turning_it_on_records_who_and_again_changes_nothing() {
        let mut cur = None;
        assert!(apply(&mut cur, true, Some("naki-2"), 10));
        assert_eq!(cur, Some(Pause { since_ms: 10, by: Some("naki-2".to_string()) }));
        // Another agent turning it on again doesn't take over the record.
        assert!(!apply(&mut cur, true, None, 20));
        assert_eq!(cur.as_ref().unwrap().by.as_deref(), Some("naki-2"));
        assert!(apply(&mut cur, false, Some("agent1"), 30));
        assert_eq!(cur, None);
        assert!(!apply(&mut cur, false, None, 40));
    }
}
