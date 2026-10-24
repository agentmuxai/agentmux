// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Whether a pid another srv recorded still belongs to that srv. A recorded
//! pid can belong to any program by now (chrome.exe, conhost.exe), so a live
//! pid alone says nothing (#1270, #5367): the process must be an
//! `agentmux-srv` binary that started no later than its claim was written.
//! Used by the slug migration's stale-lock check and by a cron run another
//! channel follows (#1988).

use std::time::SystemTime;

/// How long after a claim was written its srv may still have started: the
/// claim's time and the process's start time come from different clocks'
/// rounding.
pub const START_SLACK_SECS: u64 = 5;

/// Whether `pid` is still an srv, and, given when its claim was `written`,
/// the one that wrote it. The name rule is the launcher's (`stale_srv.rs`).
pub fn srv_alive(pid: u32, written: Option<SystemTime>) -> bool {
    #[cfg(test)]
    if let Some(srv) = test_srv::is_srv(pid) {
        return srv && crate::backend::reactive::registry::pid_alive(pid);
    }
    let target = sysinfo::Pid::from(pid as usize);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[target]), true, sysinfo::ProcessRefreshKind::nothing());
    let Some(process) = sys.process(target) else { return false };
    let written_secs = written.and_then(|w| w.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    is_srv_that_wrote(&process.name().to_string_lossy(), process.start_time(), written_secs)
}

/// [`srv_alive`]'s rule on a process's executable `name` and start time: an
/// `agentmux-srv` binary started no later than its claim was written.
pub fn is_srv_that_wrote(name: &str, started_secs: u64, written_secs: Option<u64>) -> bool {
    name.to_ascii_lowercase().starts_with("agentmux-srv") && written_secs.is_none_or(|w| started_secs <= w + START_SLACK_SECS)
}

/// Tests stand in for an srv with their own process, which isn't one by
/// name: each test thread says which pids count.
#[cfg(test)]
pub mod test_srv {
    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local!(static SRV: RefCell<Option<HashSet<u32>>> = const { RefCell::new(None) });

    /// From now on, on this thread, exactly `pids` are srv processes.
    pub fn treat_as_srv(pids: &[u32]) {
        SRV.with(|s| *s.borrow_mut() = Some(pids.iter().copied().collect()));
    }

    pub(super) fn is_srv(pid: u32) -> Option<bool> {
        SRV.with(|s| s.borrow().as_ref().map(|set| set.contains(&pid)))
    }
}
