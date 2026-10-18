// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Is a command that has printed nothing for the idle timeout stuck, or busy?
//!
//! The idle kill exists for a command stuck waiting for input that never
//! comes: a pager (`less`) in the PTY
//! (docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md). But a build or
//! test run with its output sent to a file is silent too, and it was killed in
//! the middle of its work (seen 2026-10-17, `cargo test > log`). A stuck
//! command uses no CPU and starts nothing; a busy one does. So when the
//! timeout is reached, the gate looks at the command's process tree twice,
//! a probe window apart: CPU used or a new process means busy, and the
//! timeout starts over; nothing means stuck, and the command is killed as
//! before.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use agentmux_procstats::{ProcInfo, ProcKey};

/// The longest probe window: how much later than the idle timeout a stuck
/// command is killed.
const PROBE_MAX: Duration = Duration::from_secs(30);

/// What the gate says when the command has been silent for the idle timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Still looking: ask again on the next tick.
    Probing,
    /// It used CPU or started a process: restart the idle clock.
    Busy,
    /// It did neither: kill it.
    Stuck,
}

pub(crate) struct IdleGate {
    root: Option<u32>,
    window: Duration,
    probe: Option<(Instant, Option<TreeSample>)>,
}

impl IdleGate {
    /// `root`: the command's shell process, whose tree is looked at.
    pub(crate) fn new(root: Option<u32>, idle_timeout: Duration) -> Self {
        Self { root, window: PROBE_MAX.min(idle_timeout / 2), probe: None }
    }

    /// The command printed something: any probe is moot.
    pub(crate) fn reset(&mut self) {
        self.probe = None;
    }

    /// On a tick once the command has been silent for the idle timeout:
    /// whether it is stuck and should be killed. A busy one gets
    /// `last_activity` restarted, so it has a whole timeout again.
    pub(crate) fn stuck(&mut self, last_activity: &mut Instant) -> bool {
        match self.check() {
            Verdict::Probing => false,
            Verdict::Busy => {
                tracing::info!(target: "bashwrap", "silent for the idle timeout but busy (CPU or new processes); not killing");
                *last_activity = Instant::now();
                false
            }
            Verdict::Stuck => true,
        }
    }

    /// Called on each tick once the command has been silent for the idle
    /// timeout. A tree that can't be read counts as stuck, as before the gate.
    pub(crate) fn check(&mut self) -> Verdict {
        let Some((since, before)) = &self.probe else {
            self.probe = Some((Instant::now(), self.root.and_then(sample)));
            return Verdict::Probing;
        };
        if since.elapsed() < self.window {
            return Verdict::Probing;
        }
        let after = self.root.and_then(sample);
        let verdict = match (before, &after) {
            (Some(b), Some(a)) if busy(b, a, self.window) => Verdict::Busy,
            _ => Verdict::Stuck,
        };
        self.probe = None;
        verdict
    }
}

/// The pipe path's idle watcher: every 200 ms, the time since the last byte
/// either stream read (`activity`); past `idle_timeout`, the gate decides, and
/// a stuck command gets `idle_tx`. The look at the tree is a blocking
/// snapshot, so it runs off the async runtime.
pub(crate) async fn watch(
    activity: Arc<Mutex<Instant>>,
    root: Option<u32>,
    idle_timeout: Duration,
    idle_tx: oneshot::Sender<()>,
) {
    let mut gate = Some(IdleGate::new(root, idle_timeout));
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut last = *activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut g) = gate.take() else { return };
        if last.elapsed() < idle_timeout {
            g.reset();
            gate = Some(g);
            continue;
        }
        let Ok((stuck, g, last)) = tokio::task::spawn_blocking(move || (g.stuck(&mut last), g, last)).await else {
            let _ = idle_tx.send(());
            return;
        };
        if stuck {
            let _ = idle_tx.send(());
            return;
        }
        *activity.lock().unwrap_or_else(|e| e.into_inner()) = last;
        gate = Some(g);
    }
}

/// Each process of a tree, by key, with the CPU it had used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TreeSample(BTreeMap<ProcKey, u64>);

fn sample(root: u32) -> Option<TreeSample> {
    let procs = agentmux_procstats::snapshot().ok()?;
    Some(tree_sample(&procs, root))
}

/// `root` and every process descended from it.
fn tree_sample(procs: &[ProcInfo], root: u32) -> TreeSample {
    let mut out = BTreeMap::new();
    let Some(top) = procs.iter().find(|p| p.pid == root) else { return TreeSample(out) };
    let mut stack = vec![top];
    while let Some(p) = stack.pop() {
        if out.insert(p.key(), p.cpu_ns.unwrap_or(0)).is_some() {
            continue;
        }
        stack.extend(procs.iter().filter(|c| c.is_child_of(p)));
    }
    TreeSample(out)
}

/// Busy: a process that wasn't there before, or one that used at least 1% of
/// a core over the window (at least 10 ms).
fn busy(before: &TreeSample, after: &TreeSample, window: Duration) -> bool {
    let threshold = (window.as_nanos() / 100).max(10_000_000) as u64;
    after.0.iter().any(|(key, &cpu)| match before.0.get(key) {
        None => true,
        Some(&was) => cpu.saturating_sub(was) >= threshold,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, ppid: Option<u32>, cpu_ms: u64) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            start_key: pid as u64,
            started_at_ms: Some(pid as u64),
            name: format!("p{pid}"),
            cpu_ns: Some(cpu_ms * 1_000_000),
            mem_private: None,
            mem_resident: None,
            mem_commit: None,
        }
    }

    #[test]
    fn the_tree_is_the_root_and_its_descendants_only() {
        let procs = [proc(10, Some(1), 0), proc(11, Some(10), 0), proc(12, Some(11), 0), proc(20, Some(1), 0)];
        let pids: Vec<u32> = tree_sample(&procs, 10).0.keys().map(|k| k.pid).collect();
        assert_eq!(pids, vec![10, 11, 12]);
        assert!(tree_sample(&procs, 99).0.is_empty(), "a root that's gone has no tree");
    }

    #[test]
    fn cpu_or_a_new_process_is_busy_and_nothing_is_stuck() {
        let window = Duration::from_secs(30);
        let before = tree_sample(&[proc(10, None, 100), proc(11, Some(10), 0)], 10);
        // A pager: the same processes, no CPU.
        assert!(!busy(&before, &tree_sample(&[proc(10, None, 100), proc(11, Some(10), 0)], 10), window));
        // Under 1% of a core over 30 s (300 ms) is idle noise.
        assert!(!busy(&before, &tree_sample(&[proc(10, None, 100), proc(11, Some(10), 200)], 10), window));
        // A compiler: CPU.
        assert!(busy(&before, &tree_sample(&[proc(10, None, 100), proc(11, Some(10), 5_000)], 10), window));
        // cargo starting the next rustc: a new process.
        assert!(busy(&before, &tree_sample(&[proc(10, None, 100), proc(12, Some(10), 0)], 10), window));
    }

    #[test]
    fn the_gate_probes_once_then_decides() {
        // No root to look at: after the window, stuck, as before the gate.
        let mut gate = IdleGate::new(None, Duration::from_millis(40));
        assert_eq!(gate.check(), Verdict::Probing);
        assert_eq!(gate.check(), Verdict::Probing, "inside the window");
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(gate.check(), Verdict::Stuck);
        // Output in between starts the probe over.
        assert_eq!(gate.check(), Verdict::Probing);
        gate.reset();
        assert_eq!(gate.check(), Verdict::Probing);
    }

    #[test]
    fn the_probe_window_is_at_most_half_the_timeout() {
        assert_eq!(IdleGate::new(None, Duration::from_secs(600)).window, PROBE_MAX);
        assert_eq!(IdleGate::new(None, Duration::from_secs(1)).window, Duration::from_millis(500));
    }

    /// A command that prints nothing while it works (a build with its output
    /// in a file) is not killed: here 4 s of a busy loop, silent, against a
    /// 1 s idle timeout. A stuck one still is (`bash_wrap`'s idle-kill tests).
    mod end_to_end {
        use crate::bash_wrap::{locate_bash, run_via_pipes, run_via_pty, Args};
        use crate::cwd_state::CWD_STATE_FILE_OVERRIDE_ENV;
        use crate::test_env_lock::{unique_temp_dir, EnvVarGuard, ENV_LOCK};
        use std::sync::Arc;
        use std::time::Duration;

        const BUSY_AND_SILENT: &str = "end=$((SECONDS+4)); while [ $SECONDS -lt $end ]; do :; done; echo finished";

        fn args(tool: &str) -> Args {
            Args { tool_id: tool.into(), b64_cmd: String::new(), block_id: None, declared_background: false, agent: None }
        }

        #[tokio::test]
        async fn a_silent_but_busy_command_finishes_through_pipes() {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let scratch = unique_temp_dir("idle-busy-pipes");
            let _state = EnvVarGuard::set(CWD_STATE_FILE_OVERRIDE_ENV, scratch.join("s.cwd").to_str().unwrap());
            let _idle = EnvVarGuard::set("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "1");
            let bash = locate_bash().expect("bash");
            let buffered = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
            let code = tokio::time::timeout(
                Duration::from_secs(30),
                run_via_pipes(&args("idle-busy-pipes"), BUSY_AND_SILENT, None, buffered.clone(), &bash),
            )
            .await
            .expect("finishes")
            .expect("runs");
            let out = String::from_utf8_lossy(&buffered.lock().await).into_owned();
            let _ = std::fs::remove_dir_all(&scratch);
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("finished"), "{out}");
            assert!(!out.contains("idle timeout"), "{out}");
        }

        #[tokio::test]
        async fn a_silent_but_busy_command_finishes_in_a_pty() {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let scratch = unique_temp_dir("idle-busy-pty");
            let _state = EnvVarGuard::set(CWD_STATE_FILE_OVERRIDE_ENV, scratch.join("s.cwd").to_str().unwrap());
            let _idle = EnvVarGuard::set("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "1");
            let bash = locate_bash().expect("bash");
            let size = portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
            let Ok(pair) = portable_pty::native_pty_system().openpty(size) else {
                eprintln!("skipping: PTY unavailable in this environment");
                return;
            };
            let buffered = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
            let code = tokio::time::timeout(
                Duration::from_secs(30),
                run_via_pty(&args("idle-busy-pty"), BUSY_AND_SILENT, None, buffered.clone(), &bash, pair),
            )
            .await
            .expect("finishes")
            .expect("runs");
            let out = String::from_utf8_lossy(&buffered.lock().await).into_owned();
            let _ = std::fs::remove_dir_all(&scratch);
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("finished"), "{out}");
            assert!(!out.contains("idle timeout"), "{out}");
        }
    }
}
