// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The recoveries only the launcher sees, kept for the user
//! (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.4, Phase C):
//! the window process restarted after it stopped, with or without graphics
//! acceleration; the background service restarted; and AgentMux giving up.
//!
//! Written to the same file the host keeps (`agentmux_common::incidents`)
//! before the replacement starts, so its first window lists the incident and
//! shows its notice; after a give-up, the next start does. Each `did` line is
//! something this path did.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use agentmux_common::incidents::Incident;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Start recording in `data_dir`, the folder the host records in too.
pub fn init(data_dir: &Path) {
    let _ = PATH.set(data_dir.join(agentmux_common::incidents::FILE_NAME));
}

/// Record `incident` (built by one of the functions below). Best-effort: a
/// failure is logged, never raised, since the callers are recovery paths.
pub fn record(incident: &Incident) {
    use crate::logging::log;
    log(&format!("[incident] {}: {}", incident.kind, incident.what));
    if let Some(path) = PATH.get() {
        if let Err(e) = agentmux_common::incidents::append(path, incident) {
            log(&format!("[incident] couldn't write the incident log: {e}"));
        }
    }
}

const REOPENED: &str = "AgentMux reopened your windows, tabs and panes.";
const AGENTS_KEPT: &str = "Every agent kept running: agents don't run in the window.";
const SAVED_FOR_NEXT_START: &str = "Your windows, tabs and panes are saved, and come back when you start AgentMux.";

/// Why a process exited, from its exit code: the Windows faults AgentMux sees
/// in words, any other code as it is. `None` for a plain failure exit (1),
/// which says nothing.
pub fn exit_cause(code: i32) -> Option<String> {
    let words = match code as u32 {
        1 => return None,
        0xC000_0005 => "an access violation",
        0xC000_0409 => "a failed safety check",
        0xC000_00FD => "a stack overflow",
        0xC000_0374 => "heap corruption",
        0xC000_0017 | 0xE000_0008 => return Some("It ran out of memory.".to_string()),
        0xC000_013A => return Some("It was ended with Ctrl+C or by Windows shutting it down.".to_string()),
        // TerminateProcess's usual code: Task Manager, Stop-Process, a security tool.
        0xFFFF_FFFF => return Some("Its process was ended from outside AgentMux, for example by Task Manager or a security tool.".to_string()),
        _ => return Some(format!("It exited with code {:#X}.", code as u32)),
    };
    Some(format!("A fault in its process stopped it ({words}, {:#X}).", code as u32))
}

/// The window process stopped and the launcher started it again; with
/// `safe_mode`, without graphics acceleration, because it stopped the same
/// way twice in a row.
pub fn host_restarted(code: i32, safe_mode: bool) -> Incident {
    let mut i = if safe_mode {
        let mut i = Incident::new("host_restarted_safe_mode", "AgentMux restarted without graphics acceleration", "launcher");
        i.cause = Some(format!(
            "Its window process stopped the same way twice in a row, so AgentMux turned graphics acceleration off to get it running.{}",
            exit_cause(code).map(|c| format!(" {c}")).unwrap_or_default()
        ));
        i
    } else {
        let mut i = Incident::new("host_restarted", "AgentMux restarted its window after it stopped unexpectedly", "launcher");
        i.cause = exit_cause(code);
        i
    };
    i.did = vec![REOPENED.to_string(), AGENTS_KEPT.to_string()];
    if safe_mode {
        i.did.push("Graphics acceleration comes back the next time you start AgentMux.".to_string());
    }
    i
}

/// The window process stopped because the computer ran out of memory, and
/// the launcher started it again once memory was back, without graphics
/// acceleration.
pub fn host_restarted_low_memory() -> Incident {
    let mut i = Incident::new("host_restarted_safe_mode", "AgentMux restarted after the computer ran out of memory", "launcher");
    i.cause = Some("The computer had no memory left to give its window process.".to_string());
    i.did = vec![
        "AgentMux waited for memory to free up, then reopened your windows, tabs and panes.".to_string(),
        AGENTS_KEPT.to_string(),
        "It runs without graphics acceleration, which uses less memory, until you next start it.".to_string(),
    ];
    i
}

/// Why srv stopped before the launcher started it again.
#[derive(Debug, Clone, Copy)]
pub enum SrvStop {
    /// It exited, with this code.
    Exited(i32),
    /// The computer ran out of memory; the launcher waited for it.
    LowMemory,
    /// It stopped answering, and the launcher ended it.
    Unresponsive,
}

/// srv stopped and the launcher started it again (then the window, which
/// was connected to the old one).
pub fn srv_restarted(why: SrvStop) -> Incident {
    let mut i = Incident::new("srv_restarted", "AgentMux's background service restarted", "launcher");
    i.cause = match why {
        SrvStop::Exited(code) => exit_cause(code),
        SrvStop::LowMemory => Some("The computer ran out of memory.".to_string()),
        SrvStop::Unresponsive => Some("It stopped answering, so AgentMux ended it.".to_string()),
    };
    i.did = vec![
        match why {
            SrvStop::LowMemory => "AgentMux waited for memory to free up, then started the service again.",
            _ => "AgentMux started the service again.",
        }
        .to_string(),
        REOPENED.to_string(),
    ];
    i.lost = vec!["An agent's turn that was under way may have been cut off; its conversation is saved.".to_string()];
    i
}

/// The launcher stopped trying: the computer stayed out of memory.
pub fn memory_give_up() -> Incident {
    let mut i = Incident::new("memory_give_up", "AgentMux stopped: the computer was out of memory", "launcher");
    i.cause = Some("Memory didn't free up, and starting again would only have failed again.".to_string());
    i.did = vec![SAVED_FOR_NEXT_START.to_string()];
    i
}

/// The launcher stopped trying: `what` kept stopping, `restarts` times within
/// `window_secs`.
pub fn crash_give_up(what: GaveUpOn, code: i32, restarts: usize, window_secs: u64) -> Incident {
    let (kind, title) = match what {
        GaveUpOn::Window => ("host_give_up", "AgentMux stopped: its window process kept crashing"),
        GaveUpOn::Service => ("srv_give_up", "AgentMux stopped: its background service kept crashing"),
    };
    let mut i = Incident::new(kind, title, "launcher");
    i.cause = Some(format!(
        "It stopped {restarts} times in {window_secs} seconds, so AgentMux stopped restarting it.{}",
        exit_cause(code).map(|c| format!(" The last time: {c}")).unwrap_or_default()
    ));
    i.did = vec![SAVED_FOR_NEXT_START.to_string()];
    i
}

/// Which process a give-up was about.
#[derive(Debug, Clone, Copy)]
pub enum GaveUpOn {
    Window,
    Service,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_become_words_and_an_unknown_one_is_named() {
        assert_eq!(exit_cause(0xC000_0005u32 as i32).as_deref(), Some("A fault in its process stopped it (an access violation, 0xC0000005)."));
        assert_eq!(exit_cause(0xE000_0008u32 as i32).as_deref(), Some("It ran out of memory."));
        assert_eq!(exit_cause(1), None, "a plain failure says nothing");
        // Seen in a dev build: the host ended with Stop-Process.
        assert!(exit_cause(-1).unwrap().starts_with("Its process was ended from outside AgentMux"));
        assert_eq!(exit_cause(42).as_deref(), Some("It exited with code 0x2A."));
    }

    #[test]
    fn a_restart_says_what_was_reopened_and_safe_mode_says_why_and_until_when() {
        let plain = host_restarted(0xC000_0005u32 as i32, false);
        assert_eq!(plain.kind, "host_restarted");
        assert_eq!(plain.did, vec![REOPENED.to_string(), AGENTS_KEPT.to_string()]);
        let safe = host_restarted(1, true);
        assert_eq!(safe.kind, "host_restarted_safe_mode");
        assert!(safe.cause.as_deref().unwrap().starts_with("Its window process stopped the same way twice"));
        assert!(safe.did.last().unwrap().contains("next time you start AgentMux"));
    }

    #[test]
    fn a_service_restart_says_a_turn_may_have_been_cut_off() {
        let i = srv_restarted(SrvStop::Exited(0xC000_0409u32 as i32));
        assert_eq!(i.kind, "srv_restarted");
        assert_eq!(i.cause.as_deref(), Some("A fault in its process stopped it (a failed safety check, 0xC0000409)."));
        assert_eq!(i.lost.len(), 1);
        assert_eq!(srv_restarted(SrvStop::LowMemory).cause.as_deref(), Some("The computer ran out of memory."));
        assert_eq!(srv_restarted(SrvStop::Unresponsive).cause.as_deref(), Some("It stopped answering, so AgentMux ended it."));
    }

    #[test]
    fn a_give_up_says_how_often_and_that_the_layout_is_saved() {
        let i = crash_give_up(GaveUpOn::Window, 1, 3, 60);
        assert_eq!(i.kind, "host_give_up");
        assert_eq!(i.cause.as_deref(), Some("It stopped 3 times in 60 seconds, so AgentMux stopped restarting it."));
        assert_eq!(i.did, vec![SAVED_FOR_NEXT_START.to_string()]);
        assert_eq!(crash_give_up(GaveUpOn::Service, 1, 5, 30).kind, "srv_give_up");
        assert_eq!(memory_give_up().kind, "memory_give_up");
    }

    #[test]
    fn incidents_land_in_the_data_folders_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(agentmux_common::incidents::FILE_NAME);
        agentmux_common::incidents::append(&path, &host_restarted(1, false)).unwrap();
        let got = agentmux_common::incidents::read(&path, 10);
        assert_eq!(got.len(), 1);
        assert!(got[0].evidence.as_deref().unwrap().starts_with("launcher log, "));
    }
}
