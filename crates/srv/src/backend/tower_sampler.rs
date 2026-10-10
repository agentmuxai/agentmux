// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower: CPU and memory per task, for the read-only task manager pane
//! (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md).
//!
//! A **task** is a pane and every process it started, plus one more for
//! AgentMux itself. Each sample takes one process snapshot
//! (`agentmux_procstats`) and groups it:
//!
//! 1. **The pane's tracker** (`process_tracker`): a Job Object or cgroup on
//!    an agent, which nothing it starts can leave; a best-effort scan on
//!    macOS.
//! 2. **The pane's process tree** where there is no tracker membership (a
//!    terminal on Linux or macOS): descendants of its shell.
//! 3. **Sticky membership**: a process seen in a task stays in it until it
//!    exits, even after it is reparented away from the tree.
//! 4. **AgentMux**: the backend's own ancestry while it is AgentMux's
//!    (launcher, window host) and everything under it no task claimed.
//!
//! Sampling is driven by the pane (`tower.sample`): nothing runs while no
//! Tower is open, and two panes asking at once share a sample.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentmux_procstats::{ProcInfo, ProcKey, RateMeter};

use crate::backend::process_tracker::registry::BlockMembers;
use crate::backend::tower_agentmux::Describer;
use crate::backend::process_tracker::{agent_started, TrackedProcess, TrackingConfidence};
use crate::backend::rpc_types::{TowerHost, TowerMachine, TowerProcess, TowerProcessRole, TowerSnapshot, TowerTask, TowerTaskKind};

/// `TowerTask::id` of AgentMux's own processes.
pub const AGENTMUX_TASK_ID: &str = "agentmux";

/// How often the pane asks while it is visible.
pub const INTERVAL: Duration = Duration::from_secs(2);

/// A request this soon after the last sample gets that sample: two Tower panes
/// (or a re-render) don't double the cost or shrink the CPU window.
const REUSE_WITHIN: Duration = Duration::from_millis(900);

/// A gap longer than this since the last sample starts the CPU rates over, so
/// a pane that was hidden for minutes doesn't show minutes-long averages.
const STALE_AFTER: Duration = Duration::from_secs(10);

/// Most processes one task's tree walk takes (a fork bomb shouldn't stall a
/// sample).
const MAX_TREE: usize = 2048;

/// How long a task's exited process stays listed (dimmed, with what it
/// used): long enough to see a build's compilers that a 2 s sample catches
/// once or never. SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §5.4.
const EXITED_LINGER: Duration = Duration::from_secs(60);

/// At most this many exited processes are kept, the newest: a big build
/// runs thousands of compilers a minute.
const MAX_EXITED: usize = 500;

/// A task's process that has exited: what it was when last seen.
#[derive(Clone)]
struct Exited {
    info: ProcInfo,
    task: String,
    role: TowerProcessRole,
    peak: Option<u64>,
    gone: Instant,
    gone_ms: u64,
}

/// What the sampler is told about the panes.
#[derive(Debug, Default)]
pub struct Inputs {
    /// Every block with a tracker.
    pub blocks: Vec<BlockMembers>,
    /// `(block id, its process)` for blocks registered for per-pane stats
    /// (terminal shells), the tree walk's starting points.
    pub roots: Vec<(String, u32)>,
    /// This backend's PID: where the AgentMux task is found from.
    pub own_pid: u32,
}

/// How a block is named and what kind of task it is.
#[derive(Debug, Clone)]
pub struct BlockLabel {
    pub label: String,
    pub agent: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Group {
    id: String,
    tracking: &'static str,
    cpu_time_ns: Option<u64>,
    /// Indices into the snapshot, with each one's role.
    members: Vec<(usize, TowerProcessRole)>,
}

#[derive(Debug, Default, PartialEq)]
struct Grouping {
    tasks: Vec<Group>,
    agentmux: Vec<usize>,
}

fn is_agentmux_process(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("agentmux")
}

fn tracking_label(c: TrackingConfidence) -> &'static str {
    match c {
        TrackingConfidence::High => "high",
        TrackingConfidence::BestEffort => "best_effort",
        TrackingConfidence::None => "tree",
    }
}

/// Group a snapshot into tasks (module doc). Pure: everything it needs is in
/// its arguments.
fn group(snap: &[ProcInfo], inputs: &Inputs, sticky: &HashMap<ProcKey, String>) -> Grouping {
    let by_pid: HashMap<u32, usize> = snap.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    // Parent → children, trusting a parent only if it really is one (a
    // Windows parent PID can name a newer process that reused it).
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, p) in snap.iter().enumerate() {
        if let Some(&parent) = p.ppid.and_then(|pp| by_pid.get(&pp)) {
            if p.is_child_of(&snap[parent]) {
                children.entry(parent).or_default().push(i);
            }
        }
    }
    let descendants = |root: usize, claimed: &HashMap<usize, usize>| -> Vec<usize> {
        let mut out = vec![root];
        let mut queue = VecDeque::from([root]);
        while let Some(i) = queue.pop_front() {
            for &c in children.get(&i).into_iter().flatten() {
                if out.len() >= MAX_TREE {
                    return out;
                }
                if !claimed.contains_key(&c) && !out.contains(&c) {
                    out.push(c);
                    queue.push_back(c);
                }
            }
        }
        out
    };

    let mut groups: Vec<Group> = Vec::new();
    // Snapshot index → its group's index in `groups`.
    let mut claimed: HashMap<usize, usize> = HashMap::new();

    // 1. Trackers.
    let mut blocks: Vec<&BlockMembers> = inputs.blocks.iter().collect();
    blocks.sort_by(|a, b| a.block_id.cmp(&b.block_id));
    for b in blocks {
        let members: Vec<usize> = b
            .pids
            .iter()
            .filter_map(|pid| by_pid.get(pid).copied())
            .filter(|i| !claimed.contains_key(i))
            .collect();
        // A tree whose processes all came and went between two samples still
        // has its CPU account: keep it, so that work is counted. It is shown
        // only while that account moved (`render`).
        if members.is_empty() && b.cpu_time_ns.is_none() {
            continue;
        }
        let tracked: Vec<TrackedProcess> = members
            .iter()
            .map(|&i| TrackedProcess {
                pid: snap[i].pid,
                command: snap[i].name.clone(),
                rss_bytes: 0,
                started_at_ms: 0,
                parent_pid: snap[i].ppid,
                exe: String::new(),
            })
            .collect();
        let started: HashSet<u32> = agent_started(tracked, &b.roots).into_iter().map(|p| p.pid).collect();
        let gi = groups.len();
        groups.push(Group {
            id: b.block_id.clone(),
            tracking: tracking_label(b.confidence),
            cpu_time_ns: b.cpu_time_ns,
            members: members
                .iter()
                .map(|&i| {
                    let pid = snap[i].pid;
                    let role = if b.roots.contains(&pid) {
                        TowerProcessRole::Main
                    } else if started.contains(&pid) {
                        TowerProcessRole::Started
                    } else {
                        TowerProcessRole::Support
                    };
                    (i, role)
                })
                .collect(),
        });
        for &i in &members {
            claimed.insert(i, gi);
        }
    }

    // 2. Process trees, for blocks the trackers didn't cover.
    let mut roots: Vec<&(String, u32)> = inputs.roots.iter().collect();
    roots.sort();
    for (block_id, pid) in roots {
        if groups.iter().any(|g| &g.id == block_id) {
            continue;
        }
        let Some(&root) = by_pid.get(pid) else { continue };
        if claimed.contains_key(&root) {
            continue;
        }
        let tree = descendants(root, &claimed);
        let gi = groups.len();
        groups.push(Group {
            id: block_id.clone(),
            tracking: tracking_label(TrackingConfidence::None),
            cpu_time_ns: None,
            members: tree
                .iter()
                .map(|&i| (i, if i == root { TowerProcessRole::Main } else { TowerProcessRole::Started }))
                .collect(),
        });
        for i in tree {
            claimed.insert(i, gi);
        }
    }

    // 3. Sticky: still alive, not claimed this round, its task still here.
    let by_key: HashMap<ProcKey, usize> = snap.iter().enumerate().map(|(i, p)| (p.key(), i)).collect();
    let mut sticky: Vec<(&ProcKey, &String)> = sticky.iter().collect();
    sticky.sort();
    for (key, block_id) in sticky {
        let Some(&i) = by_key.get(key) else { continue };
        if claimed.contains_key(&i) {
            continue;
        }
        let Some(gi) = groups.iter().position(|g| &g.id == block_id) else { continue };
        groups[gi].members.push((i, TowerProcessRole::Started));
        claimed.insert(i, gi);
    }

    // 4. AgentMux: up from this backend while the parent is AgentMux's, then
    //    everything under that no task claimed.
    let mut agentmux = Vec::new();
    if let Some(&me) = by_pid.get(&inputs.own_pid) {
        let mut top = me;
        for _ in 0..16 {
            let p = &snap[top];
            let Some(&parent) = p.ppid.and_then(|pp| by_pid.get(&pp)) else { break };
            if !p.is_child_of(&snap[parent]) || !is_agentmux_process(&snap[parent].name) {
                break;
            }
            top = parent;
        }
        if !claimed.contains_key(&top) {
            agentmux = descendants(top, &claimed);
        }
    }

    Grouping { tasks: groups, agentmux }
}

/// Rows a snapshot sent to another machine carries per list (busiest,
/// largest), as a remote frame does (`agentmux_remote::procs`).
pub const SHARED_TOP: usize = 100;

/// Make a snapshot fit to send to a paired device: drop the tasks `hidden`
/// names (an agent hidden from paired devices) and their processes, keep the
/// host rows matching every word of `filter`, then only the `top` busiest
/// and `top` largest of those. Totals keep counting every process.
pub fn share(snap: &mut TowerSnapshot, top: usize, filter: &str, hidden: &dyn Fn(&str) -> bool) {
    snap.remote = true;
    // Asked once per task (`hidden` reads the store), then used for every row.
    let hidden: HashSet<String> = snap
        .tasks
        .iter()
        .filter(|t| t.kind != TowerTaskKind::Agentmux && hidden(&t.id))
        .map(|t| t.id.clone())
        .collect();
    snap.tasks.retain(|t| !hidden.contains(&t.id));
    // A task's own list is capped like the host's, so one process-heavy agent
    // can't make every frame megabytes (its totals still count every process).
    for t in &mut snap.tasks {
        keep_top(&mut t.processes, top);
    }
    let Some(host) = snap.host.as_mut() else { return };
    host.processes.retain(|p| p.task.as_ref().is_none_or(|t| !hidden.contains(t)));
    // Name, PID and the task's label, as the pane's own filter matches.
    let labels: HashMap<&str, String> = snap.tasks.iter().map(|t| (t.id.as_str(), t.label.to_lowercase())).collect();
    let words: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
    host.processes.retain(|p| {
        let task = p.task.as_deref().and_then(|t| labels.get(t)).map_or("", String::as_str);
        let detail = p.detail.as_deref().unwrap_or("").to_lowercase();
        let hay = format!("{} {} {task} {detail}", p.name.to_lowercase(), p.pid);
        words.iter().all(|w| hay.contains(w.as_str()))
    });
    host.matched = host.processes.len() as u32;
    keep_top(&mut host.processes, top);
}

/// Keep the `top` busiest by CPU and the `top` largest by memory (their
/// union, in their original order).
fn keep_top(processes: &mut Vec<TowerProcess>, top: usize) {
    if processes.len() <= top {
        return;
    }
    let mut by_cpu: Vec<usize> = (0..processes.len()).collect();
    by_cpu.sort_by(|&a, &b| processes[b].cpu.unwrap_or(-1.0).total_cmp(&processes[a].cpu.unwrap_or(-1.0)));
    let mut by_mem: Vec<usize> = (0..processes.len()).collect();
    by_mem.sort_by_key(|&i| std::cmp::Reverse(processes[i].mem.map_or(-1, |m| m as i128)));
    let keep: HashSet<usize> = by_cpu.into_iter().take(top).chain(by_mem.into_iter().take(top)).collect();
    let mut i = 0;
    processes.retain(|_| {
        i += 1;
        keep.contains(&(i - 1))
    });
}

fn proc_id(p: &ProcInfo) -> String {
    format!("{}:{}", p.pid, p.start_key)
}


struct State {
    procs: RateMeter<ProcKey>,
    tasks: RateMeter<String>,
    sticky: HashMap<ProcKey, String>,
    /// What each of AgentMux's own processes is (`tower_agentmux`).
    describer: Describer,
    /// The last measurement: what a request within [`REUSE_WITHIN`] renders
    /// from, so it neither costs a second read nor shrinks the CPU window to
    /// milliseconds (one coarse clock tick over that would read as a spike).
    last: Option<Sampled>,
    /// Each live process's most private memory seen.
    peaks: HashMap<ProcKey, u64>,
    /// Tasks' processes that exited within [`EXITED_LINGER`], oldest first.
    exited: VecDeque<Exited>,
}

/// One measurement: the process table, every process's CPU rate, the grouping
/// and each task's CPU account rate. [`render`] turns it into an answer.
struct Sampled {
    at: Instant,
    ts_ms: u64,
    procs: Vec<ProcInfo>,
    rates: Vec<Option<f64>>,
    grouping: Grouping,
    account_rates: HashMap<String, Option<f64>>,
    /// AgentMux's own processes: index → what it is (`TowerProcess::detail`).
    details: HashMap<usize, String>,
    /// Each process's peak private memory, by index.
    peaks: Vec<Option<u64>>,
    /// Tasks' processes that exited within [`EXITED_LINGER`].
    exited: Vec<Exited>,
}

/// The process-wide sampler. Its state is the previous sample (for CPU
/// rates) and sticky membership.
pub struct Tower {
    state: Mutex<State>,
}

impl Default for Tower {
    fn default() -> Self {
        Self::new()
    }
}

impl Tower {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                procs: RateMeter::new(),
                tasks: RateMeter::new(),
                sticky: HashMap::new(),
                describer: Describer::default(),
                last: None,
                peaks: HashMap::new(),
                exited: VecDeque::new(),
            }),
        }
    }

    pub fn global() -> &'static Tower {
        static GLOBAL: OnceLock<Tower> = OnceLock::new();
        GLOBAL.get_or_init(Tower::new)
    }

    /// A sample (module doc). BLOCKING: reads the process table, and `labels`
    /// and `renderer` may read the store; call it off the async workers.
    /// `renderer` says what a renderer PID serves ("window Main"), for the
    /// AgentMux task's rows (`tower_agentmux::renderer_serves`).
    pub fn sample(
        &self,
        want_host: bool,
        hostname: &str,
        inputs: impl FnOnce() -> Inputs,
        labels: impl Fn(&str) -> Option<BlockLabel>,
        renderer: impl Fn(u32) -> Option<String>,
    ) -> std::io::Result<TowerSnapshot> {
        self.sample_from(agentmux_procstats::snapshot, want_host, hostname, inputs, labels, renderer)
    }

    /// [`sample`](Self::sample) with the process table read by `snapshot`.
    fn sample_from(
        &self,
        snapshot: impl FnOnce() -> std::io::Result<Vec<ProcInfo>>,
        want_host: bool,
        hostname: &str,
        inputs: impl FnOnce() -> Inputs,
        labels: impl Fn(&str) -> Option<BlockLabel>,
        renderer: impl Fn(u32) -> Option<String>,
    ) -> std::io::Result<TowerSnapshot> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = st.last.as_ref().is_some_and(|l| l.at.elapsed() < REUSE_WITHIN);
        if !fresh {
            let procs = snapshot()?;
            let now = Instant::now();
            if st.last.as_ref().is_some_and(|l| now.duration_since(l.at) > STALE_AFTER) {
                st.procs = RateMeter::new();
                st.tasks = RateMeter::new();
            }
            let sampled = advance(&mut st, procs, &inputs(), now, &agentmux_procstats::command_line_of);
            st.last = Some(sampled);
        }
        let last = st.last.as_ref().expect("measured above");
        Ok(render(last, want_host, hostname, &labels, &renderer))
    }
}

/// Measure: group the table, take every process's and task account's rate
/// since the previous measurement, and remember sticky membership.
/// `cmdline` reads a process's command line, for labelling AgentMux's own
/// processes only (`tower_agentmux`).
fn advance(
    st: &mut State,
    procs: Vec<ProcInfo>,
    inputs: &Inputs,
    now: Instant,
    cmdline: &dyn Fn(ProcKey) -> Option<String>,
) -> Sampled {
    let grouping = group(&procs, inputs, &st.sticky);
    let details = st.describer.describe(&procs, &grouping.agentmux, inputs.own_pid, cmdline);
    let (peaks, exited) = track_peaks_and_exits(st, &procs, now);
    // Every process's rate, every round, so the Host view has rates the
    // moment it is opened.
    let rates: Vec<Option<f64>> = procs
        .iter()
        .map(|p| p.cpu_ns.and_then(|ns| st.procs.sample(p.key(), ns, now)))
        .collect();
    st.procs.finish();
    let mut account_rates = HashMap::new();
    let mut sticky = HashMap::new();
    for g in &grouping.tasks {
        if let Some(ns) = g.cpu_time_ns {
            account_rates.insert(g.id.clone(), st.tasks.sample(g.id.clone(), ns, now));
        }
        for &(i, _) in &g.members {
            sticky.insert(procs[i].key(), g.id.clone());
        }
    }
    st.tasks.finish();
    st.sticky = sticky;
    Sampled {
        at: now,
        ts_ms: agentmux_common::time::now_ms() as u64,
        procs,
        rates,
        grouping,
        account_rates,
        details,
        peaks,
        exited,
    }
}

/// What a renderer row adds after its type: the window or pane it serves.
fn renderer_suffix(detail: &str, pid: u32, renderer: &dyn Fn(u32) -> Option<String>) -> Option<String> {
    (detail == "Renderer").then(|| renderer(pid)).flatten()
}

/// Update each process's peak memory, and note the tasks' members that were
/// in the previous measurement and are gone from this one. Returns the
/// peaks by index and the exited processes still within [`EXITED_LINGER`].
fn track_peaks_and_exits(st: &mut State, procs: &[ProcInfo], now: Instant) -> (Vec<Option<u64>>, Vec<Exited>) {
    let live: HashSet<ProcKey> = procs.iter().map(|p| p.key()).collect();
    // Only a recent measurement says what exited just now. The sampler runs
    // while a Tower pane is visible; after a gap (a hidden pane) what is gone
    // went at any time in it, so nothing is listed as just exited.
    let recent = st.last.as_ref().filter(|prev| now.duration_since(prev.at) <= STALE_AFTER);
    if let Some(prev) = recent {
        let gone_ms = agentmux_common::time::now_ms() as u64;
        for g in &prev.grouping.tasks {
            for &(i, role) in &g.members {
                let p = &prev.procs[i];
                if live.contains(&p.key()) {
                    continue;
                }
                let peak = st.peaks.get(&p.key()).copied().or(p.mem_private);
                st.exited.push_back(Exited { info: p.clone(), task: g.id.clone(), role, peak, gone: now, gone_ms });
            }
        }
    }
    // Gone past the linger, or (a PID reused with the same key is impossible,
    // but a process seen gone in one read can reappear in the next) back.
    st.exited.retain(|e| now.duration_since(e.gone) < EXITED_LINGER && !live.contains(&e.info.key()));
    while st.exited.len() > MAX_EXITED {
        st.exited.pop_front();
    }
    let mut peaks = HashMap::with_capacity(procs.len());
    for p in procs {
        let peak = match (st.peaks.get(&p.key()).copied(), p.mem_private) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if let Some(v) = peak {
            peaks.insert(p.key(), v);
        }
    }
    st.peaks = peaks;
    let by_index = procs.iter().map(|p| st.peaks.get(&p.key()).copied()).collect();
    (by_index, st.exited.iter().cloned().collect())
}

/// A sum of rates, or `None` when none of them is known yet (a first
/// sample), never a made-up 0.
fn sum_known(rates: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    rates.flatten().fold(None, |acc, r| Some(acc.unwrap_or(0.0) + r))
}

/// The answer from a measurement. Pure apart from `labels` and `renderer`.
fn render(
    s: &Sampled,
    want_host: bool,
    hostname: &str,
    labels: &dyn Fn(&str) -> Option<BlockLabel>,
    renderer: &dyn Fn(u32) -> Option<String>,
) -> TowerSnapshot {
    let (procs, rates, grouping) = (&s.procs, &s.rates, &s.grouping);
    let row = |i: usize, role: Option<TowerProcessRole>, task: Option<String>| {
        let p = &procs[i];
        TowerProcess {
            id: proc_id(p),
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.clone(),
            started_at_ms: p.started_at_ms,
            cpu: rates[i],
            mem: p.mem_private,
            mem_resident: p.mem_resident,
            mem_commit: p.mem_commit,
            role,
            task,
            detail: s.details.get(&i).map(|d| match renderer_suffix(d, p.pid, renderer) {
                Some(serves) => format!("{d} · {serves}"),
                None => d.clone(),
            }),
            cpu_time_ns: p.cpu_ns,
            peak_mem: s.peaks.get(i).copied().flatten(),
            exited_ms: None,
            started_by: crate::backend::tool_calls::lookup(p.pid, p.started_at_ms).map(|c| c.description),
        }
    };
    let exited_of = |task: &str| -> Vec<TowerProcess> {
        s.exited
            .iter()
            .filter(|e| e.task == task)
            .map(|e| TowerProcess {
                id: proc_id(&e.info),
                pid: e.info.pid,
                ppid: e.info.ppid,
                name: e.info.name.clone(),
                started_at_ms: e.info.started_at_ms,
                role: Some(e.role),
                cpu_time_ns: e.info.cpu_ns,
                peak_mem: e.peak,
                exited_ms: Some(e.gone_ms),
                ..Default::default()
            })
            .collect()
    };

    let mut tasks = Vec::new();
    for g in &grouping.tasks {
        let cpu = if g.cpu_time_ns.is_some() {
            s.account_rates.get(&g.id).copied().flatten()
        } else {
            sum_known(g.members.iter().map(|m| rates[m.0]))
        };
        let exited = exited_of(&g.id);
        // Nothing running, nothing used since the last sample and nothing
        // that just exited: no row.
        if g.members.is_empty() && cpu.unwrap_or(0.0) <= 0.0 && exited.is_empty() {
            continue;
        }
        let Some(label) = labels(&g.id) else { continue };
        tasks.push(TowerTask {
            id: g.id.clone(),
            kind: if label.agent { TowerTaskKind::Agent } else { TowerTaskKind::Terminal },
            label: label.label,
            tracking: g.tracking.to_string(),
            cpu,
            cpu_account: g.cpu_time_ns.is_some(),
            mem: g.members.iter().filter_map(|&(i, _)| procs[i].mem_private).sum(),
            processes: g.members.iter().map(|&(i, role)| row(i, Some(role), None)).collect(),
            exited: (!exited.is_empty()).then_some(exited),
        });
    }

    if !grouping.agentmux.is_empty() {
        tasks.push(TowerTask {
            id: AGENTMUX_TASK_ID.to_string(),
            kind: TowerTaskKind::Agentmux,
            label: "AgentMux".to_string(),
            tracking: tracking_label(TrackingConfidence::None).to_string(),
            cpu: sum_known(grouping.agentmux.iter().map(|&i| rates[i])),
            cpu_account: false,
            mem: grouping.agentmux.iter().filter_map(|&i| procs[i].mem_private).sum(),
            processes: grouping
                .agentmux
                .iter()
                .map(|&i| row(i, Some(TowerProcessRole::Main), None))
                .collect(),
            exited: None,
        });
    }

    let host = want_host.then(|| {
        let mut owner: HashMap<usize, String> = HashMap::new();
        for t in &grouping.tasks {
            for &(i, _) in &t.members {
                owner.insert(i, t.id.clone());
            }
        }
        for &i in &grouping.agentmux {
            owner.insert(i, AGENTMUX_TASK_ID.to_string());
        }
        TowerHost {
            processes: (0..procs.len()).map(|i| row(i, None, owner.get(&i).cloned())).collect(),
            unmeasured: procs.iter().filter(|p| !p.measured()).count() as u32,
            cpu: sum_known(rates.iter().copied()),
            mem: procs.iter().filter_map(|p| p.mem_private).sum(),
            total: procs.len() as u32,
            matched: procs.len() as u32,
        }
    });

    TowerSnapshot {
        ts_ms: s.ts_ms,
        hostname: hostname.to_string(),
        os: std::env::consts::OS.to_string(),
        cpu_count: agentmux_procstats::cpu_count() as u32,
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
        interval_ms: INTERVAL.as_millis() as u32,
        remote: false,
        tasks,
        // Already measured for every process each round (`advance`), so the
        // totals cost nothing even when the full list isn't asked for.
        machine: Some(TowerMachine {
            cpu: sum_known(rates.iter().copied()),
            mem: procs.iter().filter_map(|p| p.mem_private).sum(),
            processes: procs.len() as u32,
        }),
        host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        st: &mut State,
        procs: &[ProcInfo],
        inputs: &Inputs,
        now: Instant,
        want_host: bool,
        hostname: &str,
        labels: &dyn Fn(&str) -> Option<BlockLabel>,
    ) -> TowerSnapshot {
        let sampled = advance(st, procs.to_vec(), inputs, now, &|_| None);
        let snap = render(&sampled, want_host, hostname, labels, &|_| None);
        // As `Tower::sample_from` does: the next measurement compares to it.
        st.last = Some(sampled);
        snap
    }

    fn state() -> State {
        State {
            procs: RateMeter::new(),
            tasks: RateMeter::new(),
            sticky: HashMap::new(),
            describer: Describer::default(),
            last: None,
            peaks: HashMap::new(),
            exited: VecDeque::new(),
        }
    }

    fn p(pid: u32, ppid: u32, name: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid).filter(|&x| x != 0),
            start_key: pid as u64 * 10,
            started_at_ms: Some(pid as u64 * 10),
            name: name.to_string(),
            cpu_ns: Some(0),
            mem_private: Some(1000),
            mem_resident: Some(2000),
            mem_commit: None,
        }
    }

    fn tracked(block: &str, pids: &[u32], roots: &[u32]) -> BlockMembers {
        BlockMembers {
            block_id: block.to_string(),
            pids: pids.to_vec(),
            roots: roots.iter().copied().collect(),
            confidence: TrackingConfidence::High,
            cpu_time_ns: Some(0),
        }
    }

    fn pids_of(snap: &[ProcInfo], idx: &[usize]) -> Vec<u32> {
        let mut v: Vec<u32> = idx.iter().map(|&i| snap[i].pid).collect();
        v.sort();
        v
    }

    fn task<'a>(g: &'a Grouping, id: &str) -> &'a Group {
        g.tasks.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("no task {id}: {g:?}"))
    }

    /// launcher → srv → claude (job) → mcp, bash → node; launcher → cef host
    /// → renderer. A terminal: srv → pwsh → vim (no tracker).
    fn machine() -> Vec<ProcInfo> {
        vec![
            p(1, 0, "init"),
            p(100, 1, "agentmux-launcher.exe"),
            p(110, 100, "agentmux-srv.exe"),
            p(120, 100, "agentmux-cef.exe"),
            p(121, 120, "agentmux-cef.exe"),
            p(200, 110, "claude.exe"),
            p(201, 200, "agentmux-mcp.exe"),
            p(202, 200, "bash.exe"),
            p(203, 202, "node.exe"),
            p(300, 110, "pwsh.exe"),
            p(301, 300, "vim.exe"),
            p(900, 1, "explorer.exe"),
        ]
    }

    #[test]
    fn tracker_members_tree_and_agentmux_are_grouped_with_roles() {
        let snap = machine();
        let inputs = Inputs {
            blocks: vec![tracked("agent-a", &[200, 201, 202, 203], &[200])],
            roots: vec![("term-b".into(), 300)],
            own_pid: 110,
        };
        let g = group(&snap, &inputs, &HashMap::new());
        let a = task(&g, "agent-a");
        let role = |pid: u32| a.members.iter().find(|(i, _)| snap[*i].pid == pid).unwrap().1;
        assert_eq!(role(200), TowerProcessRole::Main);
        assert_eq!(role(201), TowerProcessRole::Support, "the MCP server is the agent's plumbing");
        assert_eq!(role(202), TowerProcessRole::Started);
        assert_eq!(role(203), TowerProcessRole::Started);
        assert_eq!(a.tracking, "high");

        let b = task(&g, "term-b");
        assert_eq!(pids_of(&snap, &b.members.iter().map(|m| m.0).collect::<Vec<_>>()), vec![300, 301]);
        assert_eq!(b.tracking, "tree");

        // AgentMux: the launcher (srv's AgentMux parent) and everything under
        // it that no task claimed; not init, not explorer.
        assert_eq!(pids_of(&snap, &g.agentmux), vec![100, 110, 120, 121]);
    }

    #[test]
    fn a_tracker_covers_a_block_its_tree_would_have() {
        // The terminal has a Job Object too (Windows): its members win.
        let snap = machine();
        let inputs = Inputs {
            blocks: vec![tracked("term-b", &[300, 301], &[300])],
            roots: vec![("term-b".into(), 300)],
            own_pid: 110,
        };
        let g = group(&snap, &inputs, &HashMap::new());
        assert_eq!(g.tasks.len(), 1);
        assert_eq!(task(&g, "term-b").tracking, "high");
    }

    #[test]
    fn a_reparented_process_stays_in_its_task() {
        // vim's shell exited; vim was reparented to init.
        let mut snap = machine();
        snap.retain(|x| x.pid != 300);
        snap.iter_mut().find(|x| x.pid == 301).unwrap().ppid = Some(1);
        let inputs = Inputs { blocks: vec![], roots: vec![("term-b".into(), 300)], own_pid: 110 };
        // No root, no task — unless the task is still around through another
        // member; here it isn't, so vim belongs to nobody.
        let g = group(&snap, &inputs, &HashMap::from([(snap.iter().find(|x| x.pid == 301).unwrap().key(), "term-b".to_string())]));
        assert!(g.tasks.is_empty());

        // With the shell alive (a sibling of vim was reparented), sticky
        // membership keeps the reparented one in the task.
        let mut snap = machine();
        snap.push(p(302, 1, "server.exe"));
        let sticky = HashMap::from([(snap.last().unwrap().key(), "term-b".to_string())]);
        let inputs = Inputs { blocks: vec![], roots: vec![("term-b".into(), 300)], own_pid: 110 };
        let g = group(&snap, &inputs, &sticky);
        let members: Vec<usize> = task(&g, "term-b").members.iter().map(|m| m.0).collect();
        assert_eq!(pids_of(&snap, &members), vec![300, 301, 302]);
    }

    #[test]
    fn a_reused_parent_pid_is_not_trusted() {
        // 301's recorded parent (300) exited and a NEWER process got PID 300.
        let mut snap = machine();
        let newer = snap.iter_mut().find(|x| x.pid == 300).unwrap();
        newer.started_at_ms = Some(999_999);
        newer.start_key = 999_999;
        let inputs = Inputs { blocks: vec![], roots: vec![("term-b".into(), 300)], own_pid: 110 };
        let g = group(&snap, &inputs, &HashMap::new());
        let members: Vec<usize> = task(&g, "term-b").members.iter().map(|m| m.0).collect();
        assert_eq!(pids_of(&snap, &members), vec![300], "vim is not the newer process's child");
    }

    #[test]
    fn agentmux_stops_at_a_parent_that_is_not_agentmux() {
        // Run straight from a terminal (task dev): srv's parent is a shell.
        let snap = vec![p(1, 0, "init"), p(50, 1, "bash"), p(110, 50, "agentmux-srv"), p(111, 110, "agentmux-srv")];
        let g = group(&snap, &Inputs { own_pid: 110, ..Default::default() }, &HashMap::new());
        assert_eq!(pids_of(&snap, &g.agentmux), vec![110, 111]);
    }

    fn label(id: &str) -> Option<BlockLabel> {
        Some(BlockLabel { label: format!("label {id}"), agent: id.starts_with("agent") })
    }

    /// A build's compiler that exits between samples stays in its task's
    /// `exited` list for a minute, with all the CPU it used and its peak
    /// memory, then goes; it never counts as running.
    #[test]
    fn a_task_process_that_exits_lingers_a_minute_with_what_it_used() {
        let mut st = state();
        let t0 = Instant::now();
        let mut snap = machine();
        let inputs = Inputs { blocks: vec![tracked("agent-a", &[200, 201, 202, 203], &[200])], roots: vec![], own_pid: 110 };
        let node = |snap: &mut Vec<ProcInfo>| snap.iter_mut().find(|x| x.pid == 203).unwrap().clone();
        let set = |snap: &mut Vec<ProcInfo>, mem: u64, cpu_ns: u64| {
            let n = snap.iter_mut().find(|x| x.pid == 203).unwrap();
            n.mem_private = Some(mem);
            n.cpu_ns = Some(cpu_ns);
        };
        set(&mut snap, 400, 1_000_000_000);
        build(&mut st, &snap, &inputs, t0, false, "host", &label);
        set(&mut snap, 100, 2_000_000_000);
        let second = build(&mut st, &snap, &inputs, t0 + Duration::from_secs(2), false, "host", &label);
        let task = |s: &TowerSnapshot| s.tasks.iter().find(|t| t.id == "agent-a").cloned().unwrap();
        let live = task(&second).processes.into_iter().find(|p| p.pid == 203).unwrap();
        assert_eq!((live.peak_mem, live.cpu_time_ns), (Some(400), Some(2_000_000_000)), "the peak, not the current");
        assert!(task(&second).exited.is_none());

        let gone_key = node(&mut snap).key();
        snap.retain(|x| x.pid != 203);
        let third = build(&mut st, &snap, &inputs, t0 + Duration::from_secs(4), false, "host", &label);
        let t = task(&third);
        assert!(t.processes.iter().all(|p| p.pid != 203), "an exited process isn't running");
        let exited = t.exited.unwrap();
        assert_eq!(exited.len(), 1);
        let e = &exited[0];
        assert_eq!(e.id, format!("{}:{}", gone_key.pid, gone_key.start_key));
        assert_eq!((e.peak_mem, e.cpu_time_ns, e.cpu, e.mem), (Some(400), Some(2_000_000_000), None, None));
        assert_eq!(e.role, Some(TowerProcessRole::Started));
        assert!(e.exited_ms.is_some());

        let later = build(&mut st, &snap, &inputs, t0 + Duration::from_secs(4) + EXITED_LINGER, false, "host", &label);
        assert!(task(&later).exited.is_none(), "gone after the linger");
    }

    /// After a gap (no Tower pane visible), what went missing in it isn't
    /// listed as just exited.
    #[test]
    fn processes_gone_during_a_gap_are_not_listed_as_just_exited() {
        let mut st = state();
        let t0 = Instant::now();
        let mut snap = machine();
        let inputs = Inputs { blocks: vec![tracked("agent-a", &[200, 201, 202, 203], &[200])], roots: vec![], own_pid: 110 };
        build(&mut st, &snap, &inputs, t0, false, "host", &label);
        snap.retain(|x| x.pid != 203);
        let after_gap = build(&mut st, &snap, &inputs, t0 + STALE_AFTER + Duration::from_secs(1), false, "host", &label);
        let task = after_gap.tasks.iter().find(|t| t.id == "agent-a").unwrap();
        assert!(task.exited.is_none(), "nothing just exited after a gap");
    }

    #[test]
    fn rates_come_from_the_job_account_and_the_process_deltas() {
        let mut st = state();
        let t0 = Instant::now();
        let mut snap = machine();
        let mut inputs = Inputs {
            blocks: vec![tracked("agent-a", &[200, 201, 202, 203], &[200])],
            roots: vec![("term-b".into(), 300)],
            own_pid: 110,
        };
        let first = build(&mut st, &snap, &inputs, t0, true, "host", &label);
        assert!(first.tasks.iter().all(|t| t.cpu.is_none()), "no rates on the first sample");
        assert_eq!(first.host.as_ref().unwrap().cpu, None, "nor a made-up 0% for the machine");

        // One second later: node used 0.5 s; vim 0.25 s; the agent's job
        // account grew by 2 s (a build that already exited used most of it).
        snap.iter_mut().find(|x| x.pid == 203).unwrap().cpu_ns = Some(500_000_000);
        snap.iter_mut().find(|x| x.pid == 301).unwrap().cpu_ns = Some(250_000_000);
        inputs.blocks[0].cpu_time_ns = Some(2_000_000_000);
        let second = build(&mut st, &snap, &inputs, t0 + Duration::from_secs(1), true, "host", &label);
        let t = |id: &str| second.tasks.iter().find(|t| t.id == id).unwrap();
        assert!((t("agent-a").cpu.unwrap() - 2.0).abs() < 1e-9, "the job account, exited members included");
        assert!(t("agent-a").cpu_account);
        assert!((t("term-b").cpu.unwrap() - 0.25).abs() < 1e-9, "the tree's per-process sum");
        assert_eq!(t("agent-a").kind, TowerTaskKind::Agent);
        assert_eq!(t("term-b").kind, TowerTaskKind::Terminal);
        assert_eq!(t("agent-a").mem, 4000);
        assert_eq!(t(AGENTMUX_TASK_ID).kind, TowerTaskKind::Agentmux);

        let host = second.host.unwrap();
        assert_eq!(host.processes.len(), snap.len());
        assert!((host.cpu.unwrap() - 0.75).abs() < 1e-9);
        let node = host.processes.iter().find(|x| x.pid == 203).unwrap();
        assert_eq!(node.task.as_deref(), Some("agent-a"));
        assert!(host.processes.iter().find(|x| x.pid == 900).unwrap().task.is_none());
        assert_eq!(node.id, "203:2030");

        // The machine's totals come with every sample, the full list or not,
        // and are the host list's own.
        let machine = second.machine.clone().unwrap();
        assert_eq!(machine.cpu, host.cpu);
        assert_eq!(machine.mem, host.mem);
        assert_eq!(machine.processes as usize, snap.len());
        // (`build` measures again: a sample at the same instant has no CPU
        // interval, so only what doesn't depend on one is compared.)
        let without_list = build(&mut st, &snap, &inputs, t0 + Duration::from_secs(2), false, "host", &label);
        assert!(without_list.host.is_none());
        let again = without_list.machine.unwrap();
        assert_eq!((again.mem, again.processes), (machine.mem, machine.processes));
        assert_eq!(first.machine.unwrap().cpu, None, "no made-up 0% on the first sample");
    }

    /// A build that started and finished between two samples: the tree has
    /// no live process, but its Job account grew, and that CPU is reported.
    #[test]
    fn a_tree_whose_processes_all_exited_still_reports_their_cpu() {
        let mut st = state();
        let t0 = Instant::now();
        let mut inputs = Inputs { blocks: vec![tracked("agent-a", &[], &[200])], roots: vec![], own_pid: 110 };
        inputs.blocks[0].cpu_time_ns = Some(1_000_000_000);
        let first = build(&mut st, &machine(), &inputs, t0, false, "h", &label);
        assert!(first.tasks.iter().all(|t| t.id != "agent-a"), "nothing measured yet, nothing running: no row");
        inputs.blocks[0].cpu_time_ns = Some(3_000_000_000);
        let second = build(&mut st, &machine(), &inputs, t0 + Duration::from_secs(1), false, "h", &label);
        let a = second.tasks.iter().find(|t| t.id == "agent-a").expect("its CPU is reported");
        assert!((a.cpu.unwrap() - 2.0).abs() < 1e-9);
        assert!(a.processes.is_empty());
        // Idle afterwards: the row goes away again.
        let third = build(&mut st, &machine(), &inputs, t0 + Duration::from_secs(2), false, "h", &label);
        assert!(third.tasks.iter().all(|t| t.id != "agent-a"));
    }

    /// What a paired device gets: no hidden agent, no row of its processes,
    /// the filter applied, only the busiest and largest.
    #[test]
    fn a_shared_snapshot_hides_hidden_agents_and_keeps_the_top() {
        let mut st = state();
        let inputs = Inputs {
            blocks: vec![tracked("agent-a", &[200, 201, 202, 203], &[200]), tracked("agent-b", &[300, 301], &[300])],
            roots: vec![],
            own_pid: 110,
        };
        let mut snap = build(&mut st, &machine(), &inputs, Instant::now(), true, "h", &label);
        share(&mut snap, 2, "", &|id| id == "agent-b");
        assert!(snap.remote);
        assert!(snap.tasks.iter().all(|t| t.id != "agent-b"));
        let host = snap.host.as_ref().unwrap();
        assert!(host.processes.iter().all(|p| p.task.as_deref() != Some("agent-b")));
        assert!(host.processes.len() <= 4, "two lists of two");
        assert_eq!(host.total, machine().len() as u32, "totals still count everything");
        let a = snap.tasks.iter().find(|t| t.id == "agent-a").unwrap();
        assert!(a.processes.len() <= 4, "a task's own list is capped too");
        assert_eq!(a.mem, 4000, "but its totals count every process");

        let mut snap = build(&mut st, &machine(), &inputs, Instant::now(), true, "h", &label);
        share(&mut snap, 100, "node", &|_| false);
        assert_eq!(snap.host.as_ref().unwrap().matched, 1);
        assert_eq!(snap.host.unwrap().processes[0].name, "node.exe");

        // By the task's label, as the pane's filter matches.
        let mut snap = build(&mut st, &machine(), &inputs, Instant::now(), true, "h", &label);
        share(&mut snap, 100, "agent-b", &|_| false);
        let names: Vec<String> = snap.host.unwrap().processes.into_iter().map(|p| p.name).collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn a_block_without_a_label_is_left_out() {
        let mut st = state();
        let inputs = Inputs { blocks: vec![tracked("gone", &[200], &[200])], roots: vec![], own_pid: 110 };
        let s = build(&mut st, &machine(), &inputs, Instant::now(), false, "h", &|_| None);
        assert!(s.tasks.iter().all(|t| t.id != "gone"));
        assert!(s.host.is_none());
    }

    /// Opening the Host view right after a Tasks sample renders the same
    /// measurement: no second read, so no CPU window of a few milliseconds.
    #[test]
    fn host_rows_added_within_the_reuse_window_come_from_the_same_measurement() {
        let tower = Tower::new();
        let reads = std::cell::Cell::new(0);
        let table = || {
            reads.set(reads.get() + 1);
            Ok(machine())
        };
        let inputs = || Inputs { own_pid: 110, ..Default::default() };
        let tasks_only = tower.sample_from(table, false, "h", inputs, label, |_| None).unwrap();
        assert!(tasks_only.host.is_none());
        let with_host = tower.sample_from(table, true, "h", inputs, label, |_| None).unwrap();
        assert_eq!(reads.get(), 1, "one read for both");
        assert_eq!(with_host.ts_ms, tasks_only.ts_ms);
        assert_eq!(with_host.host.unwrap().processes.len(), machine().len());
    }

    /// The real sampler on this machine finds this test process as part of
    /// "AgentMux"-less grouping without error, and a second request within
    /// the reuse window is the same sample.
    #[test]
    fn a_real_sample_and_its_reuse() {
        let tower = Tower::new();
        let inputs = || Inputs { own_pid: std::process::id(), ..Default::default() };
        let a = tower.sample(true, "h", inputs, |_| None, |_| None).unwrap();
        let b = tower.sample(false, "h", inputs, |_| None, |_| None).unwrap();
        assert_eq!(a.ts_ms, b.ts_ms, "reused");
        assert!(a.host.as_ref().unwrap().processes.iter().any(|x| x.pid == std::process::id()));
        assert!(a.cpu_count >= 1);
    }
}
