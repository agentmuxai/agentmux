// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Notices a build-slot holder that has stopped working: its process tree has used no CPU for
//! [`IDLE_AFTER_SECS`] (a hung test, a prompt waiting for input, a lost network call), while
//! everyone else waits in the queue. Never stops anything.
//!
//! Git Bash cannot do this itself: it sees no CPU time for native programs and reports pid 1 for a
//! parent that is one. So the script records its Windows process id (`winpid` in the owner file)
//! and srv, once a minute while a slot is held, adds up the CPU of that process and everything
//! below it. When it has not moved, srv
//!
//! - writes `<slot>/idle` (`build-slot status` prints "idle for N min, no CPU"), and
//! - tells the holder's agent once, then at most once an hour while it stays idle.
//!
//! The decisions are pure ([`Watch::observe`], [`tree_cpu_ms`]) and the process list is a
//! parameter of [`check`], so they are tested without a clock or a running build.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use agentmux_common::build_slot::{self as slot, SlotHolder};

/// A holder whose tree used no CPU for this long is idle.
pub const IDLE_AFTER_SECS: u64 = 10 * 60;
/// While it stays idle, its agent hears of it at most this often.
pub const RENOTIFY_SECS: u64 = 60 * 60;
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// The script's own heartbeat (a subshell that touches a file every 20 s) is in the tree, and a
/// fork under Git Bash costs it a little CPU each time. That much, plus [`NOISE_MS_PER_SEC`] for
/// every second since the CPU last moved, is not work.
const NOISE_FLOOR_MS: u64 = 500;
const NOISE_MS_PER_SEC: u64 = 5;

/// One process, as far as the check needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    /// CPU time used since it started, user plus kernel, in milliseconds.
    pub cpu_ms: u64,
}

/// The CPU time of `root` and every process below it, or `None` when `root` is not running.
pub fn tree_cpu_ms(procs: &[Proc], root: u32) -> Option<u64> {
    procs.iter().find(|p| p.pid == root)?;
    let mut seen: HashSet<u32> = HashSet::from([root]);
    let mut queue = vec![root];
    let mut total = 0u64;
    while let Some(pid) = queue.pop() {
        for p in procs {
            if p.pid == pid {
                total = total.saturating_add(p.cpu_ms);
            }
            // A parent link can be stale (a reused pid): `seen` keeps it from looping.
            if p.parent == Some(pid) && seen.insert(p.pid) {
                queue.push(p.pid);
            }
        }
    }
    Some(total)
}

/// What the last look at the holder found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Working, or not known to be idle.
    Busy,
    /// No CPU since `since` (epoch seconds). `notify`: its agent should be told now.
    Idle { since: u64, notify: bool },
}

#[derive(Debug)]
struct Run {
    token: String,
    /// The tree's CPU when it last moved by more than noise, and when that was.
    base_cpu_ms: u64,
    base_at: u64,
    notified_at: Option<u64>,
}

/// What srv remembers between looks: the run being watched.
#[derive(Debug, Default)]
pub struct Watch {
    run: Option<Run>,
}

impl Watch {
    /// Forgets the run: the slot is free, or the holder cannot be watched.
    pub fn forget(&mut self) {
        self.run = None;
    }

    /// One look: `cpu_ms` is the holder tree's CPU now (`None` when it could not be read) and
    /// `now` epoch seconds.
    pub fn observe(&mut self, token: &str, cpu_ms: Option<u64>, now: u64) -> Step {
        let Some(cpu) = cpu_ms else {
            self.run = None;
            return Step::Busy;
        };
        let run = match &mut self.run {
            Some(run) if run.token == token => run,
            other => other.insert(Run { token: token.to_string(), base_cpu_ms: cpu, base_at: now, notified_at: None }),
        };
        let allowed = NOISE_FLOOR_MS + now.saturating_sub(run.base_at) * NOISE_MS_PER_SEC;
        // Less than before means a child ended: something happened.
        if cpu < run.base_cpu_ms || cpu - run.base_cpu_ms > allowed {
            *run = Run { token: run.token.clone(), base_cpu_ms: cpu, base_at: now, notified_at: None };
            return Step::Busy;
        }
        if now.saturating_sub(run.base_at) < IDLE_AFTER_SECS {
            return Step::Busy;
        }
        let notify = run.notified_at.is_none_or(|at| now.saturating_sub(at) >= RENOTIFY_SECS);
        if notify {
            run.notified_at = Some(now);
        }
        Step::Idle { since: run.base_at, notify }
    }
}

/// A holder found idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idle {
    pub holder: SlotHolder,
    pub since: u64,
    pub notify: bool,
}

/// One look at the slot under `root` at `now` (epoch seconds). `processes` lists the machine's
/// processes and is only called when a slot is held. Keeps `<slot>/idle` in step with what it
/// finds, and returns the holder when it is idle.
pub fn check(root: &Path, watch: &mut Watch, now: u64, processes: impl FnOnce() -> Vec<Proc>) -> Option<Idle> {
    let Some(holder) = slot::read_holder(root, now) else {
        watch.forget();
        return None;
    };
    // A script from before version 9 wrote no Windows id, and a run with no token cannot be told
    // from the next one: neither can be watched.
    let Some(pid) = holder.process_id().filter(|_| !holder.token.is_empty()) else {
        watch.forget();
        return None;
    };
    let cpu = tree_cpu_ms(&processes(), pid);
    match watch.observe(&holder.token, cpu, now) {
        Step::Busy => {
            slot::clear_idle(root);
            None
        }
        Step::Idle { since, notify } => {
            if let Err(e) = slot::write_idle(root, &holder.token, since) {
                tracing::debug!(error = %e, "build slot: could not write the idle note");
            }
            Some(Idle { holder, since, notify })
        }
    }
}

/// What the holder's agent is told.
pub fn note_text(idle: &Idle, now: u64) -> String {
    let minutes = now.saturating_sub(idle.since) / 60;
    let process = match idle.holder.process_id() {
        Some(pid) => format!("its process id is {pid}, and what it started runs below it"),
        None => "by its process id, which `BUILD_SLOT_VERBOSE=1 build-slot status` shows".to_string(),
    };
    let command: String = idle.holder.command.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!(
        "The build you started in the build slot has used no CPU for {minutes} min: {command}\n\
         It may be stuck (a hung test, a prompt waiting for input, a lost network call), and anyone queued behind it is waiting. \
         `build-slot status` shows it. AgentMux has not stopped it: if it is stuck, end it ({process}) so the slot is free. \
         You hear of it again at most once an hour while it stays idle."
    )
}

fn snapshot(sys: &mut sysinfo::System) -> Vec<Proc> {
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::nothing().with_cpu());
    sys.processes()
        .iter()
        .map(|(pid, p)| Proc { pid: pid.as_u32(), parent: p.parent().map(|p| p.as_u32()), cpu_ms: p.accumulated_cpu_time() })
        .collect()
}

/// Starts the once-a-minute look. Only the srv that runs the host's cleanups does it, so two
/// AgentMux channels on one machine do not both write the note or message the agent.
pub fn install() {
    if !crate::backend::build_cache_service::this_srv_may_clean() {
        return;
    }
    let spawned = std::thread::Builder::new().name("build-slot-idle".into()).spawn(|| {
        let (mut watch, mut sys) = (Watch::default(), sysinfo::System::new());
        loop {
            std::thread::sleep(CHECK_EVERY);
            let Some(root) = slot::lock_dir() else { return };
            let now = agentmux_common::time::now_secs_u64();
            let Some(idle) = check(&root, &mut watch, now, || snapshot(&mut sys)).filter(|i| i.notify) else { continue };
            let handler = crate::backend::reactive::get_global_handler();
            match handler.deliver_system_note(&idle.holder.agent, &note_text(&idle, now)) {
                Ok(()) => tracing::info!(agent = %idle.holder.agent, idle_min = (now - idle.since) / 60, "build slot: told the holder it is idle"),
                Err(e) => tracing::debug!(agent = %idle.holder.agent, error = %e, "build slot: the idle holder was not told"),
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "build slot: idle watch not started");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: Option<u32>, cpu_ms: u64) -> Proc {
        Proc { pid, parent, cpu_ms }
    }

    #[test]
    fn the_tree_adds_the_root_and_everything_below_it_and_nothing_else() {
        let procs = [p(1, None, 5), p(10, Some(1), 100), p(11, Some(10), 200), p(12, Some(10), 300), p(20, Some(1), 7000)];
        assert_eq!(tree_cpu_ms(&procs, 10), Some(600), "the root, a child and a grandchild; not its sibling or parent");
        assert_eq!(tree_cpu_ms(&procs, 99), None, "a root that is not running");
        let looped = [p(1, Some(2), 1), p(2, Some(1), 2)];
        assert_eq!(tree_cpu_ms(&looped, 1), Some(3), "a parent cycle (a reused pid) does not loop");
    }

    const T0: u64 = 1_000_000;

    #[test]
    fn a_tree_that_uses_no_cpu_is_idle_after_the_wait_and_told_once_an_hour() {
        let mut w = Watch::default();
        assert_eq!(w.observe("a", Some(5000), T0), Step::Busy, "the first look only takes the base");
        assert_eq!(w.observe("a", Some(5000), T0 + 9 * 60), Step::Busy, "not yet");
        assert_eq!(w.observe("a", Some(5000), T0 + 10 * 60), Step::Idle { since: T0, notify: true });
        assert_eq!(w.observe("a", Some(5000), T0 + 11 * 60), Step::Idle { since: T0, notify: false }, "told already");
        assert_eq!(w.observe("a", Some(5000), T0 + 10 * 60 + 59 * 60), Step::Idle { since: T0, notify: false });
        assert_eq!(w.observe("a", Some(5000), T0 + 10 * 60 + 60 * 60), Step::Idle { since: T0, notify: true }, "an hour after the first note");
    }

    #[test]
    fn a_busy_command_is_never_idle() {
        let mut w = Watch::default();
        let mut cpu = 1000;
        for minute in 0..60 {
            cpu += 20_000; // a compile using a core for a fifth of the minute
            assert_eq!(w.observe("a", Some(cpu), T0 + minute * 60), Step::Busy, "minute {minute}");
        }
    }

    #[test]
    fn the_scripts_own_heartbeat_is_not_work_but_a_real_burst_is() {
        let mut w = Watch::default();
        w.observe("a", Some(1000), T0);
        // The heartbeat's forks: 30 ms every 20 s is 90 ms a minute.
        assert!(matches!(w.observe("a", Some(1000 + 900), T0 + 10 * 60), Step::Idle { notify: true, .. }));
        // Ten seconds of CPU in one minute is a build that woke up.
        assert_eq!(w.observe("a", Some(1000 + 900 + 10_000), T0 + 11 * 60), Step::Busy);
        assert_eq!(w.observe("a", Some(1000 + 900 + 10_000), T0 + 12 * 60), Step::Busy, "the wait starts over");
        assert!(matches!(w.observe("a", Some(1000 + 900 + 10_000), T0 + 21 * 60), Step::Idle { since, notify: true } if since == T0 + 11 * 60));
    }

    #[test]
    fn a_child_that_ends_counts_as_activity() {
        let mut w = Watch::default();
        w.observe("a", Some(50_000), T0);
        assert_eq!(w.observe("a", Some(10_000), T0 + 10 * 60), Step::Busy, "the total fell: a process ended");
    }

    #[test]
    fn a_new_run_or_an_unreadable_tree_starts_over() {
        let mut w = Watch::default();
        w.observe("a", Some(5000), T0);
        assert!(matches!(w.observe("a", Some(5000), T0 + 10 * 60), Step::Idle { .. }));
        assert_eq!(w.observe("b", Some(5000), T0 + 11 * 60), Step::Busy, "another holder: not idle for the first one's minutes");
        assert_eq!(w.observe("b", Some(5000), T0 + 12 * 60), Step::Busy);
        assert_eq!(w.observe("b", None, T0 + 30 * 60), Step::Busy, "the tree could not be read");
        assert_eq!(w.observe("b", Some(5000), T0 + 31 * 60), Step::Busy, "and the wait began again");
    }

    /// A held slot whose heartbeat was last touched at `alive_at` (epoch seconds): the looks below
    /// pretend to be minutes apart, so the heartbeat is dated after the last of them.
    fn holder_slot(root: &Path, owner: &str, alive_at: u64) {
        let dir = root.join("slot");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("owner"), owner).unwrap();
        let beat = std::fs::File::create(dir.join("heartbeat")).unwrap();
        beat.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(alive_at)).unwrap();
    }

    /// Both the Windows id and the pid are 700: `check` looks for the one this platform uses.
    const OWNER: &str = "agent=lazo\npid=700\nwinpid=700\ntoken=tok\nstart=1\ncmd=cargo test -p srv\n";

    #[test]
    fn check_writes_the_idle_note_and_removes_it_when_the_holder_works_again() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let t0 = agentmux_common::time::now_secs_u64();
        holder_slot(root, OWNER, t0 + 30 * 60);
        let idle_file = root.join("slot").join("idle");
        let mut w = Watch::default();
        let quiet = || vec![p(700, None, 100), p(701, Some(700), 4000)];
        assert_eq!(check(root, &mut w, t0, quiet), None, "the first look");
        assert!(!idle_file.exists());
        let idle = check(root, &mut w, t0 + 11 * 60, quiet).expect("11 minutes with no CPU");
        assert_eq!((idle.since, idle.notify, idle.holder.agent.as_str()), (t0, true, "lazo"));
        let text = std::fs::read_to_string(&idle_file).unwrap();
        assert_eq!(text, format!("token=tok\nidle_since={t0}\n"));
        let busy = || vec![p(700, None, 100), p(701, Some(700), 90_000)];
        assert_eq!(check(root, &mut w, t0 + 12 * 60, busy), None);
        assert!(!idle_file.exists(), "work removes the note");
    }

    #[test]
    fn a_free_slot_or_a_holder_that_cannot_be_watched_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut w = Watch::default();
        let never = || -> Vec<Proc> { panic!("no slot is held: the machine's processes are not listed") };
        assert_eq!(check(root, &mut w, agentmux_common::time::now_secs_u64(), never), None);
        holder_slot(root, "agent=lazo\npid=1\ntoken=tok\nstart=1\ncmd=x\n", agentmux_common::time::now_secs_u64());
        let never = || -> Vec<Proc> { panic!("no usable process id: the processes are not listed") };
        assert_eq!(check(root, &mut w, agentmux_common::time::now_secs_u64(), never), None, "a script from before version 9 wrote no Windows id");
    }

    #[test]
    fn the_note_names_the_command_and_says_nothing_was_stopped() {
        let holder = SlotHolder { agent: "lazo".into(), command: "cargo test -p srv\nsecond line".into(), pid: Some(4321), winpid: Some(4321), ..Default::default() };
        let text = note_text(&Idle { holder, since: T0, notify: true }, T0 + 12 * 60 + 5);
        assert!(text.contains("12 min") && text.contains("cargo test -p srv second line"), "got {text}");
        assert!(text.contains("has not stopped it"));
        assert!(text.contains("process id is 4321"), "the note gives the id to end: {text}");
    }
}
