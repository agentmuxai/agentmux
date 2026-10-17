// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What went wrong and what AgentMux did about it, kept for the user
//! (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.1, §4.2).
//!
//! The host records an incident when it handles one (a window's renderer
//! crashing, running out of memory or hanging; quitting because no window is
//! left), and the frontend shows each one the user hasn't closed as a toast
//! that stays until they do. Before this, every recovery path in §3.3 of the
//! spec logged and told the user nothing. The launcher writes the cases only
//! it sees to the same file (`agentmux_common::incidents`, which keeps its
//! format).

use std::path::PathBuf;
use std::sync::OnceLock;

pub use agentmux_common::incidents::Incident;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// How many incidents `list` hands the frontend.
const LIST_MAX: usize = 50;

/// Start recording in `dir` (the instance's data folder). Idempotent.
pub fn init(dir: &std::path::Path) {
    let _ = PATH.set(dir.join(agentmux_common::incidents::FILE_NAME));
}

/// A new incident of the host's, now.
fn incident(kind: &str, what: &str) -> Incident {
    Incident::new(kind, what, "host")
}

/// Append `incident`. Best-effort: a failure is logged, never raised, since
/// the callers are recovery paths.
pub fn record(incident: &Incident) {
    tracing::warn!(target: "incidents", kind = %incident.kind, id = %incident.id, what = %incident.what, "incident recorded");
    if let Some(path) = PATH.get() {
        if let Err(e) = agentmux_common::incidents::append(path, incident) {
            tracing::warn!(target: "incidents", error = %e, "couldn't write the incident log");
        }
    }
}

/// Record `incident` and tell every open window, so one still open shows it
/// now rather than on its next start.
pub fn record_and_announce(state: &crate::state::AppState, incident: &Incident) {
    record(incident);
    if let Ok(payload) = serde_json::to_value(incident) {
        crate::events::emit_event_to_top_level_windows(state, "incident-recorded", &payload);
    }
}

/// The user closed `id`'s notice: append the acknowledgement and tell every
/// window, which closes its copy.
pub fn acknowledge(state: &crate::state::AppState, id: &str) {
    if let Some(path) = PATH.get() {
        if let Err(e) = agentmux_common::incidents::append_ack(path, id, agentmux_common::time::now_ms_u64()) {
            tracing::warn!(target: "incidents", error = %e, "couldn't write the incident log");
        }
    }
    crate::events::emit_event_to_top_level_windows(state, "incident-acknowledged", &serde_json::json!({ "id": id }));
}

/// The newest incidents, newest first, each with its acknowledgement.
pub fn list() -> Vec<Incident> {
    PATH.get().map(|p| agentmux_common::incidents::read(p, LIST_MAX)).unwrap_or_default()
}


/// A renderer stopped (`client::crash_recovery`). Recorded for an AgentMux
/// window only, never a web page in a browser pane or a hidden pool window
/// (what the browser is, from the host's registry), and at most once a
/// minute, since a crash loop is one incident to the user.
pub fn renderer_terminated(state: &crate::state::AppState, browser_id: i32, hung: bool, oom: bool, detail: &str) {
    use cef::ImplBrowser;
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_MS: AtomicU64 = AtomicU64::new(0);
    let is_window = state
        .host_state
        .lock()
        .browsers
        .values()
        .find(|h| h.browser.identifier() == browser_id)
        .is_some_and(|h| matches!(h.kind, crate::state::BrowserKind::TopLevel { is_pool: false }));
    if !is_window {
        return;
    }
    let now = agentmux_common::time::now_ms_u64();
    let last = LAST_MS.load(Ordering::Relaxed);
    if last != 0 && now < last + 60_000 {
        return;
    }
    LAST_MS.store(now, Ordering::Relaxed);
    let incident = renderer_incident(hung, oom, detail);
    record_and_announce(state, &incident);
}

fn renderer_incident(hung: bool, oom: bool, detail: &str) -> Incident {
    let (kind, what) = if hung {
        ("renderer_hung", "An AgentMux window stopped responding")
    } else if oom {
        ("renderer_oom", "An AgentMux window ran out of memory")
    } else {
        ("renderer_crashed", "An AgentMux window crashed")
    };
    let mut incident = incident(kind, what);
    incident.cause = if hung {
        Some("Its page stopped answering, so AgentMux stopped it.".to_string())
    } else {
        plain_cause(detail)
    };
    incident.did = vec![
        "Every agent kept running: agents don't run in the window.".to_string(),
        "Your tabs and panes are kept, and come back when the window reloads.".to_string(),
    ];
    incident
}

/// Why a renderer stopped, in plain words, from the reason Chromium gave: a
/// result or exit code (`RESULT_CODE_KILLED`, `STATUS_ACCESS_VIOLATION`) or a
/// sentence. A code it has no words for is still named, after a sentence
/// that says it is Chromium's.
fn plain_cause(detail: &str) -> Option<String> {
    let detail = detail.trim();
    if detail.is_empty() {
        return None;
    }
    let is_code = detail.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !is_code {
        return Some(detail.chars().take(200).collect());
    }
    let words = match detail {
        "RESULT_CODE_KILLED" => "Its process was ended from outside AgentMux, for example by Task Manager or a security tool.",
        "RESULT_CODE_HUNG" => "It stopped responding.",
        "RESULT_CODE_KILLED_BAD_MESSAGE" => "Chromium stopped it after it sent an invalid message.",
        "STATUS_NO_MEMORY" | "STATUS_COMMITMENT_LIMIT" | "RESULT_CODE_OUT_OF_MEMORY" => "It ran out of memory.",
        "STATUS_ACCESS_VIOLATION" | "STATUS_STACK_BUFFER_OVERRUN" | "STATUS_HEAP_CORRUPTION" | "STATUS_BREAKPOINT"
        | "STATUS_ILLEGAL_INSTRUCTION" | "STATUS_STACK_OVERFLOW" => {
            return Some(format!("A fault in its process stopped it ({detail})."));
        }
        _ => return Some(format!("Chromium reported its process ended ({detail}).")),
    };
    Some(words.to_string())
}

/// Whether the quit watchdog's exit with no window visible is an incident:
/// the host still counts a window open and nothing asked to close it. With
/// no window registered, or a drain under way, it is a quit the user asked
/// for (the last window closed), not one to report.
pub fn is_unexpected_quit(registered: usize, draining: bool) -> bool {
    registered > 0 && !draining
}

/// The quit watchdog is about to end the app with no window left while the
/// host still counted one open (`wrr::win_event`, when `is_unexpected_quit`):
/// a window went without AgentMux closing it. Written before the exit; there
/// is no window to tell.
pub fn quit_no_window() {
    let mut incident = incident("quit_no_window", "AgentMux quit because its window closed unexpectedly");
    incident.cause = Some("The window closed without AgentMux being asked to close it.".to_string());
    incident.did = vec!["Your agents' conversations are saved, and continue when you open the agents again.".to_string()];
    record(&incident);
}

/// IPC: the incidents, for a window that just opened and for the status
/// bar's history.
pub fn incidents_list() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "incidents": list() }))
}

/// IPC: the user closed an incident's notice.
pub fn incident_acknowledge(state: &crate::state::AppState, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = args.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).ok_or("incident_acknowledge: missing id")?;
    acknowledge(state, id);
    Ok(serde_json::json!(true))
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn a_renderer_incident_says_what_stopped_why_and_what_was_kept() {
        let hung = renderer_incident(true, false, "ignored");
        assert_eq!((hung.kind.as_str(), hung.what.as_str()), ("renderer_hung", "An AgentMux window stopped responding"));
        assert!(hung.cause.as_deref().unwrap().contains("stopped answering"));
        let oom = renderer_incident(false, true, "  ");
        assert_eq!(oom.kind, "renderer_oom");
        assert_eq!(oom.cause, None, "no cause is guessed");
        let crashed = renderer_incident(false, false, "GPU process crashed");
        assert_eq!((crashed.kind.as_str(), crashed.cause.as_deref()), ("renderer_crashed", Some("GPU process crashed")));
        assert!(crashed.did.iter().any(|d| d.contains("Every agent kept running")));
    }

    /// Seen in a dev build: a renderer ended with Stop-Process came back as
    /// `RESULT_CODE_KILLED`, and the notice showed the code.
    #[test]
    fn chromiums_codes_become_plain_words_and_an_unknown_one_is_still_named() {
        let cause = |d: &str| renderer_incident(false, false, d).cause;
        assert_eq!(cause("RESULT_CODE_KILLED").as_deref(), Some("Its process was ended from outside AgentMux, for example by Task Manager or a security tool."));
        assert_eq!(cause("STATUS_NO_MEMORY").as_deref(), Some("It ran out of memory."));
        assert_eq!(cause("STATUS_ACCESS_VIOLATION").as_deref(), Some("A fault in its process stopped it (STATUS_ACCESS_VIOLATION)."));
        assert_eq!(cause("RESULT_CODE_GPU_DEAD_ON_ARRIVAL").as_deref(), Some("Chromium reported its process ended (RESULT_CODE_GPU_DEAD_ON_ARRIVAL)."));
        assert_eq!(cause(" ").as_deref(), None);
    }

    #[test]
    fn only_a_window_gone_without_being_closed_is_an_unexpected_quit() {
        assert!(is_unexpected_quit(1, false), "a live window nobody closed");
        assert!(!is_unexpected_quit(0, false), "no window left: the last one was closed");
        assert!(!is_unexpected_quit(0, true), "a quit draining after the last window closed");
        assert!(!is_unexpected_quit(2, true), "a drain under way");
    }

}
