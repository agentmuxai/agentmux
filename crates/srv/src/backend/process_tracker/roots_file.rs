// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's side of `agentmux_common::agent_roots`: the running agent CLIs, each
//! the leader of its own process group, written to `<data_dir>/agent-roots.json`
//! so the launcher can end them if srv dies without closing them.
//!
//! Recorded when an agent CLI is tracked ([`super::registry::track_spawned_agent`]),
//! pruned on the registry poller's tick once a CLI has exited, and deleted by
//! [`finish`] at srv's clean exit when no CLI is left.

use std::path::PathBuf;

use agentmux_common::agent_roots::{self, AgentRoots, Root};
use parking_lot::Mutex;

struct State {
    data_dir: PathBuf,
    file: AgentRoots,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Record a freshly spawned agent CLI.
pub fn record(pid: u32) {
    let Some(start) = agentmux_common::process::process_start_time(pid) else { return };
    let mut guard = STATE.lock();
    let state = guard.get_or_insert_with(|| State {
        data_dir: crate::backend::base::get_mux_data_dir(),
        file: AgentRoots { srv_pid: std::process::id(), ..AgentRoots::default() },
    });
    state.file.roots.retain(|r| r.pid != pid);
    state.file.roots.push(Root { pid, start });
    save(state);
}

/// Drop the CLIs that have exited. Called from the registry poller.
pub fn prune() {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else { return };
    if prune_exited(&mut state.file) {
        save(state);
    }
}

/// srv's clean exit, after every agent was closed: delete the file, or, if a
/// CLI outlived the teardown's cap, keep it (pruned) for the launcher.
pub fn finish() {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else { return };
    prune_exited(&mut state.file);
    if state.file.roots.is_empty() {
        agent_roots::remove(&state.data_dir);
    } else {
        tracing::warn!(left = state.file.roots.len(), "[process-tracker] agent CLIs still running at exit; left for the launcher");
        save(state);
    }
}

/// Remove every root that is no longer the process recorded (exited, or its
/// pid reused). True when anything was removed.
fn prune_exited(file: &mut AgentRoots) -> bool {
    let before = file.roots.len();
    file.roots
        .retain(|r| agentmux_common::process::process_start_time(r.pid) == Some(r.start));
    file.roots.len() != before
}

fn save(state: &mut State) {
    #[cfg(target_os = "linux")]
    {
        state.file.cgroup = super::cgroup_linux::root().map(|p| p.to_path_buf());
    }
    if let Err(e) = agent_roots::write(&state.data_dir, &state.file) {
        tracing::warn!(error = %e, "[process-tracker] could not write {}", agent_roots::FILE_NAME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_exited_and_reused_pids_only() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let start = agentmux_common::process::process_start_time(child.id()).unwrap();
        let mut file = AgentRoots {
            srv_pid: 1,
            cgroup: None,
            roots: vec![Root { pid: child.id(), start }, Root { pid: child.id(), start: start + 1 }],
        };
        assert!(prune_exited(&mut file), "the reused pid is dropped");
        assert_eq!(file.roots, [Root { pid: child.id(), start }]);
        assert!(!prune_exited(&mut file), "nothing more while it runs");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(prune_exited(&mut file));
        assert!(file.roots.is_empty());
    }
}
