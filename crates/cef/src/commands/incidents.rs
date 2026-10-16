// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What went wrong and what AgentMux did about it, kept for the user
//! (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.1, §4.2).
//!
//! The host records an incident when it handles one (a window's renderer
//! crashing, running out of memory or hanging; quitting because no window is
//! left), and the frontend shows each one the user hasn't closed as a toast
//! that stays until they do. Before this, every recovery path in §3.3 of the
//! spec logged and told the user nothing.
//!
//! An append-only JSON-lines file in the instance's data folder, next to the
//! background audit log: an incident is one line, closing its notice appends
//! an acknowledgement line. Written synchronously, because the quit watchdog
//! records just before the process exits; incidents are rare, and each write
//! is one small append.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// The file keeps about this many lines; past twice that it is cut back.
const KEEP_LINES: usize = 200;
/// How many incidents `list` hands the frontend.
const LIST_MAX: usize = 50;

/// Start recording in `dir` (the instance's data folder). Idempotent.
pub fn init(dir: &std::path::Path) {
    let _ = PATH.set(dir.join("incidents.jsonl"));
}

/// One incident, as the user is told about it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub id: String,
    pub at_ms: u64,
    /// A closed set: `renderer_crashed`, `renderer_oom`, `renderer_hung`,
    /// `quit_no_window`.
    pub kind: String,
    /// What happened, in plain words.
    pub what: String,
    /// Why, when it is known; never guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// What AgentMux did that keeps the user's work: only what this path did.
    #[serde(default)]
    pub did: Vec<String>,
    /// What may not have survived, when this path knows.
    #[serde(default)]
    pub lost: Vec<String>,
    /// The host log and when, for a bug report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Set by `list` from the acknowledgement lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at_ms: Option<u64>,
}

impl Incident {
    pub fn new(kind: &str, what: impl Into<String>) -> Self {
        let at_ms = agentmux_common::time::now_ms_u64();
        Incident {
            id: format!("{at_ms}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            at_ms,
            kind: kind.to_string(),
            what: what.into(),
            cause: None,
            did: Vec::new(),
            lost: Vec::new(),
            evidence: Some(format!("host log, {}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"))),
            acknowledged_at_ms: None,
        }
    }
}

/// One line of the file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Line {
    Ack { ack: String, at_ms: u64 },
    Incident(Incident),
}

/// Append `incident`. Best-effort: a failure is logged, never raised, since
/// the callers are recovery paths.
pub fn record(incident: &Incident) {
    tracing::warn!(target: "incidents", kind = %incident.kind, id = %incident.id, what = %incident.what, "incident recorded");
    if let Some(path) = PATH.get() {
        append(path, &Line::Incident(incident.clone()));
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
        append(path, &Line::Ack { ack: id.to_string(), at_ms: agentmux_common::time::now_ms_u64() });
    }
    crate::events::emit_event_to_top_level_windows(state, "incident-acknowledged", &serde_json::json!({ "id": id }));
}

/// The newest incidents, newest first, each with its acknowledgement.
pub fn list() -> Vec<Incident> {
    PATH.get().map(|p| read(p)).unwrap_or_default()
}

fn read(path: &std::path::Path) -> Vec<Incident> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut incidents: Vec<Incident> = Vec::new();
    let mut acks: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    // A torn last line (a crash mid-append) costs only itself.
    for line in text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()) {
        match line {
            Line::Ack { ack, at_ms } => {
                acks.insert(ack, at_ms);
            }
            Line::Incident(i) => incidents.push(i),
        }
    }
    for i in &mut incidents {
        i.acknowledged_at_ms = acks.get(&i.id).copied();
    }
    incidents.reverse();
    incidents.truncate(LIST_MAX);
    incidents
}

fn append(path: &std::path::Path, line: &Line) {
    let Ok(json) = serde_json::to_string(line) else {
        return;
    };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| writeln!(f, "{json}"));
    if let Err(e) = written {
        tracing::warn!(target: "incidents", error = %e, "couldn't write the incident log");
        return;
    }
    compact(path);
}

/// Keep the last `KEEP_LINES` lines once the file is past twice that.
fn compact(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= KEEP_LINES * 2 {
        return;
    }
    let kept = lines[lines.len() - KEEP_LINES..].join("\n") + "\n";
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, kept).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
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
    let mut incident = Incident::new(kind, what);
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
    let mut incident = Incident::new("quit_no_window", "AgentMux quit because its window closed unexpectedly");
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
    fn incidents_read_back_newest_first_with_their_acknowledgements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incidents.jsonl");
        let mut a = Incident::new("renderer_crashed", "A window crashed and was reloaded");
        a.did = vec!["Every agent kept running".into()];
        let b = Incident::new("quit_no_window", "AgentMux quit because no window was left");
        append(&path, &Line::Incident(a.clone()));
        append(&path, &Line::Incident(b.clone()));
        append(&path, &Line::Ack { ack: a.id.clone(), at_ms: 99 });
        // A torn last line costs only itself.
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"id\":\"torn").unwrap();
        let got = read(&path);
        assert_eq!(got.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), vec!["quit_no_window", "renderer_crashed"]);
        assert_eq!(got[1].acknowledged_at_ms, Some(99));
        assert_eq!(got[0].acknowledged_at_ms, None);
        assert_eq!(got[1].did, vec!["Every agent kept running".to_string()]);
    }

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

    #[test]
    fn the_file_is_cut_back_to_its_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incidents.jsonl");
        for n in 0..(KEEP_LINES * 2 + 1) {
            append(&path, &Line::Incident(Incident::new("renderer_crashed", format!("n{n}"))));
        }
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(lines, KEEP_LINES);
        assert_eq!(read(&path).first().map(|i| i.what.as_str()), Some(format!("n{}", KEEP_LINES * 2).as_str()));
    }
}
