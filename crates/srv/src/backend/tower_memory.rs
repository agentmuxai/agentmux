// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux's total memory, and what it is made of
//! (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §2, §5).
//!
//! Everything AgentMux started is in exactly one of four parts:
//!
//! - **Core**: its own programs: the launcher, srv, the browser host and its
//!   GPU and service processes.
//! - **Panes**: what draws and hosts the panes: the window renderers, and the
//!   terminals with what runs in them.
//! - **Agents**: each agent's tree, split into its CLI, what runs for it, and
//!   what it started.
//! - **Not placed**: the processes under AgentMux's launcher that no pane
//!   claims (the sampler's AgentMux group holds every unclaimed descendant of
//!   the launcher, not only AgentMux's own programs). Shown by name, never in
//!   Core.
//!
//! Pure: it reads the snapshot the sampler already built, so it costs one pass
//! over the rows and every number is the one the other views show.

use std::collections::HashMap;

use crate::backend::rpc_types::{
    TowerMemory, TowerMemoryChild, TowerMemoryPart, TowerMemoryPartId, TowerMemoryRow, TowerProcess,
    TowerProcessKind, TowerProcessRole, TowerSnapshot, TowerTaskKind,
};

/// The parts, in the order they are shown and always sent, even at zero.
const PARTS: [TowerMemoryPartId; 4] = [
    TowerMemoryPartId::Core,
    TowerMemoryPartId::Panes,
    TowerMemoryPartId::Agents,
    TowerMemoryPartId::NotPlaced,
];

/// One row being added up.
struct Row {
    key: String,
    label: String,
    kind: Option<TowerProcessKind>,
    task: Option<String>,
    mem: u64,
    processes: u32,
    children: Vec<TowerMemoryChild>,
}

impl Row {
    fn new(key: String, label: String, kind: Option<TowerProcessKind>, task: Option<String>) -> Self {
        Row { key, label, kind, task, mem: 0, processes: 0, children: Vec::new() }
    }

    fn add(&mut self, p: &TowerProcess) {
        self.mem += p.mem.unwrap_or(0);
        self.processes += 1;
    }
}

/// A part's rows, by key, in the order they were first seen.
#[derive(Default)]
struct Rows {
    order: Vec<String>,
    by_key: HashMap<String, Row>,
}

impl Rows {
    fn row(&mut self, key: &str, make: impl FnOnce() -> Row) -> &mut Row {
        if !self.by_key.contains_key(key) {
            self.order.push(key.to_string());
            self.by_key.insert(key.to_string(), make());
        }
        self.by_key.get_mut(key).expect("just inserted")
    }

    fn into_part(self, part: TowerMemoryPartId) -> TowerMemoryPart {
        let mut rows: Vec<TowerMemoryRow> = {
            let mut by_key = self.by_key;
            self.order
                .iter()
                .filter_map(|k| by_key.remove(k))
                .map(|r| TowerMemoryRow {
                    key: r.key,
                    label: r.label,
                    mem: r.mem,
                    processes: r.processes,
                    kind: r.kind,
                    task: r.task,
                    children: r.children,
                })
                .collect()
        };
        // Biggest first; the label breaks a tie so the order is stable.
        rows.sort_by(|a, b| b.mem.cmp(&a.mem).then_with(|| a.label.cmp(&b.label)));
        TowerMemoryPart {
            part,
            mem: rows.iter().map(|r| r.mem).sum(),
            processes: rows.iter().map(|r| r.processes).sum(),
            rows,
        }
    }
}

/// What an AgentMux program is called in the breakdown: the sampler's label
/// for it, in plainer words where that helps.
fn core_label(detail: &str) -> &str {
    match detail {
        "Main process" => "Browser host",
        "Server" => "Server (srv)",
        "GPU" => "GPU process",
        other => other,
    }
}

/// A label as a key: lower case, anything but letters and digits a dash.
fn slug(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// The role's line under an agent.
fn role_line(role: Option<TowerProcessRole>) -> (&'static str, &'static str) {
    match role {
        Some(TowerProcessRole::Main) => ("cli", "CLI"),
        Some(TowerProcessRole::Support) => ("helpers", "Helpers (MCP servers, shell hooks)"),
        Some(TowerProcessRole::Started) | None => ("tools", "Tools it started"),
    }
}

/// Total AgentMux's memory and split it into parts. Only this computer's own
/// snapshot makes sense here (another machine's tasks are a capped list).
pub fn build(snap: &TowerSnapshot) -> TowerMemory {
    let mut core = Rows::default();
    let mut panes = Rows::default();
    let mut agents = Rows::default();
    let mut not_placed = Rows::default();
    let mut unreadable = 0u32;
    let mut processes = 0u32;

    for t in &snap.tasks {
        match t.kind {
            TowerTaskKind::Agent => {
                let row = agents.row(&format!("agent:{}", t.id), || {
                    Row::new(
                        format!("agent:{}", t.id),
                        t.label.clone(),
                        Some(TowerProcessKind::Agents),
                        Some(t.id.clone()),
                    )
                });
                let mut children: Vec<TowerMemoryChild> = Vec::new();
                for p in &t.processes {
                    processes += 1;
                    unreadable += p.mem.is_none() as u32;
                    row.add(p);
                    let (id, label) = role_line(p.role);
                    let key = format!("agent:{}:{id}", t.id);
                    match children.iter_mut().find(|c| c.key == key) {
                        Some(c) => {
                            c.mem += p.mem.unwrap_or(0);
                            c.processes += 1;
                        }
                        None => children.push(TowerMemoryChild {
                            key,
                            label: label.to_string(),
                            mem: p.mem.unwrap_or(0),
                            processes: 1,
                        }),
                    }
                }
                // CLI, helpers, then tools, whichever of them it has.
                children.sort_by_key(|c| match c.key.rsplit(':').next() {
                    Some("cli") => 0,
                    Some("helpers") => 1,
                    _ => 2,
                });
                row.children = children;
            }
            TowerTaskKind::Terminal => {
                let row = panes.row(&format!("terminal:{}", t.id), || {
                    Row::new(
                        format!("terminal:{}", t.id),
                        t.label.clone(),
                        Some(TowerProcessKind::Shells),
                        Some(t.id.clone()),
                    )
                });
                for p in &t.processes {
                    processes += 1;
                    unreadable += p.mem.is_none() as u32;
                    row.add(p);
                }
            }
            TowerTaskKind::Agentmux => {
                for p in &t.processes {
                    processes += 1;
                    unreadable += p.mem.is_none() as u32;
                    match p.detail.as_deref() {
                        // A renderer draws a window (or waits in the pool).
                        Some(d) if d.starts_with("Renderer") => {
                            let label = if d == "Renderer" { "Renderer (not matched to a window)" } else { d };
                            panes
                                .row(&format!("renderer:{}", slug(d)), || {
                                    Row::new(
                                        format!("renderer:{}", slug(d)),
                                        label.to_string(),
                                        Some(TowerProcessKind::Agentmux),
                                        None,
                                    )
                                })
                                .add(p);
                        }
                        Some(d) => {
                            let label = core_label(d.split(" · ").next().unwrap_or(d));
                            core.row(&format!("core:{}", slug(label)), || {
                                Row::new(
                                    format!("core:{}", slug(label)),
                                    label.to_string(),
                                    Some(TowerProcessKind::Agentmux),
                                    None,
                                )
                            })
                            .add(p);
                        }
                        // Not one of AgentMux's own: it is here because no
                        // pane claimed it. By what it is, never in Core.
                        None => {
                            let key = format!("np:{}", slug(&p.name));
                            not_placed
                                .row(&key, || Row::new(key.clone(), p.name.clone(), p.kind, None))
                                .add(p);
                        }
                    }
                }
            }
        }
    }

    let parts: Vec<TowerMemoryPart> = PARTS
        .iter()
        .zip([core, panes, agents, not_placed])
        .map(|(&id, rows)| rows.into_part(id))
        .collect();
    TowerMemory {
        total: parts.iter().map(|p| p.mem).sum(),
        processes,
        unreadable,
        parts,
        other_instances: snap.machine.as_ref().and_then(|m| m.agentmux_elsewhere),
        metric: snap.memory_metric.clone(),
        growth: Vec::new(),
        history: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{TowerMachine, TowerTask};

    const MB: u64 = 1024 * 1024;

    fn proc(name: &str, mem: Option<u64>, role: Option<TowerProcessRole>, detail: Option<&str>) -> TowerProcess {
        TowerProcess {
            id: format!("{name}:{}", mem.unwrap_or(0)),
            name: name.to_string(),
            mem,
            role,
            detail: detail.map(str::to_string),
            ..Default::default()
        }
    }

    fn task(id: &str, kind: TowerTaskKind, label: &str, processes: Vec<TowerProcess>) -> TowerTask {
        TowerTask {
            id: id.to_string(),
            kind,
            label: label.to_string(),
            tracking: "high".to_string(),
            cpu: None,
            cpu_account: false,
            mem: processes.iter().filter_map(|p| p.mem).sum(),
            processes,
            exited: None,
        }
    }

    fn snapshot(tasks: Vec<TowerTask>) -> TowerSnapshot {
        TowerSnapshot {
            ts_ms: 1,
            hostname: "narko".to_string(),
            os: "windows".to_string(),
            cpu_count: 8,
            memory_metric: "private working set".to_string(),
            interval_ms: 2000,
            remote: false,
            tasks,
            machine: None,
            host: None,
            panes: None,
            memory: None,
        }
    }

    fn part(m: &TowerMemory, id: TowerMemoryPartId) -> &TowerMemoryPart {
        m.parts.iter().find(|p| p.part == id).expect("every part is sent")
    }

    fn fixture() -> TowerSnapshot {
        use TowerProcessRole::{Main, Started, Support};
        snapshot(vec![
            task(
                "block-a",
                TowerTaskKind::Agent,
                "Loap",
                vec![
                    proc("claude.exe", Some(300 * MB), Some(Main), None),
                    proc("node.exe", Some(120 * MB), Some(Main), None),
                    proc("agentmux-mcp.exe", Some(10 * MB), Some(Support), None),
                    proc("cargo.exe", Some(60 * MB), Some(Started), None),
                    proc("rustc.exe", Some(500 * MB), Some(Started), None),
                ],
            ),
            task(
                "block-t",
                TowerTaskKind::Terminal,
                "Terminal",
                vec![proc("pwsh.exe", Some(40 * MB), Some(Main), None), proc("git.exe", Some(5 * MB), Some(Started), None)],
            ),
            task(
                "agentmux",
                TowerTaskKind::Agentmux,
                "AgentMux",
                vec![
                    proc("agentmux-launcher.exe", Some(20 * MB), Some(Main), Some("Launcher")),
                    proc("agentmux-srv.exe", Some(150 * MB), Some(Main), Some("Server")),
                    proc("agentmux-cef.exe", Some(180 * MB), Some(Main), Some("Main process")),
                    proc("agentmux-cef.exe", Some(400 * MB), Some(Main), Some("GPU")),
                    proc("agentmux-cef.exe", Some(160 * MB), Some(Main), Some("Renderer · window Loap's workspace")),
                    proc("agentmux-cef.exe", Some(350 * MB), Some(Main), Some("Renderer · pool window (idle)")),
                    proc("claude.exe", Some(140 * MB), Some(Main), None),
                    proc("claude.exe", Some(100 * MB), Some(Main), None),
                    proc("qemu-system-x86_64.exe", Some(120 * MB), Some(Main), None),
                ],
            ),
        ])
    }

    #[test]
    fn four_parts_in_order_that_add_up_to_the_total() {
        let m = build(&fixture());
        let ids: Vec<_> = m.parts.iter().map(|p| p.part).collect();
        assert_eq!(
            ids,
            [
                TowerMemoryPartId::Core,
                TowerMemoryPartId::Panes,
                TowerMemoryPartId::Agents,
                TowerMemoryPartId::NotPlaced
            ]
        );
        let all: u64 = fixture().tasks.iter().map(|t| t.mem).sum();
        assert_eq!(m.total, all);
        assert_eq!(m.parts.iter().map(|p| p.mem).sum::<u64>(), m.total);
        assert_eq!(m.processes, 16);
        assert_eq!(m.parts.iter().map(|p| p.processes).sum::<u32>(), m.processes);
        assert_eq!(m.metric, "private working set");
    }

    #[test]
    fn an_empty_part_is_still_sent_at_zero() {
        let m = build(&snapshot(vec![]));
        assert_eq!(m.parts.len(), 4);
        assert!(m.parts.iter().all(|p| p.mem == 0 && p.rows.is_empty()));
        assert_eq!(m.total, 0);
    }

    /// The bug behind "AgentMux contains Claude": an agent's CLI that no pane
    /// claimed is in the sampler's AgentMux group, and must not be Core.
    #[test]
    fn a_stray_claude_is_not_placed_and_never_core() {
        let m = build(&fixture());
        let core = part(&m, TowerMemoryPartId::Core);
        assert!(core.rows.iter().all(|r| !r.label.contains("claude") && !r.label.contains("qemu")));
        let np = part(&m, TowerMemoryPartId::NotPlaced);
        let claude = np.rows.iter().find(|r| r.key == "np:claude-exe").expect("grouped by name");
        assert_eq!((claude.mem, claude.processes), (240 * MB, 2));
        assert!(np.rows.iter().any(|r| r.label == "qemu-system-x86_64.exe"));
        assert_eq!(np.mem, 360 * MB);
    }

    #[test]
    fn core_is_agentmux_s_own_programs_by_what_they_are() {
        let m = build(&fixture());
        let core = part(&m, TowerMemoryPartId::Core);
        let labels: Vec<_> = core.rows.iter().map(|r| r.label.as_str()).collect();
        // Biggest first.
        assert_eq!(labels, ["GPU process", "Browser host", "Server (srv)", "Launcher"]);
        assert_eq!(core.mem, (400 + 180 + 150 + 20) * MB);
        assert!(core.rows.iter().all(|r| r.kind == Some(TowerProcessKind::Agentmux)));
    }

    #[test]
    fn panes_are_the_window_renderers_and_each_terminal_with_what_runs_in_it() {
        let m = build(&fixture());
        let panes = part(&m, TowerMemoryPartId::Panes);
        let labels: Vec<_> = panes.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Renderer · pool window (idle)", "Renderer · window Loap's workspace", "Terminal"]);
        let terminal = panes.rows.iter().find(|r| r.task.as_deref() == Some("block-t")).expect("terminal row");
        assert_eq!((terminal.mem, terminal.processes), (45 * MB, 2));
        assert_eq!(terminal.kind, Some(TowerProcessKind::Shells));
    }

    #[test]
    fn a_renderer_with_no_window_is_still_counted_and_says_so() {
        let t = task(
            "agentmux",
            TowerTaskKind::Agentmux,
            "AgentMux",
            vec![proc("agentmux-cef.exe", Some(10 * MB), None, Some("Renderer"))],
        );
        let m = build(&snapshot(vec![t]));
        let panes = part(&m, TowerMemoryPartId::Panes);
        assert_eq!(panes.rows[0].label, "Renderer (not matched to a window)");
        assert_eq!(m.total, 10 * MB);
    }

    #[test]
    fn an_agent_is_one_row_split_into_cli_helpers_and_tools() {
        let m = build(&fixture());
        let agents = part(&m, TowerMemoryPartId::Agents);
        assert_eq!(agents.rows.len(), 1);
        let loap = &agents.rows[0];
        assert_eq!((loap.key.as_str(), loap.label.as_str()), ("agent:block-a", "Loap"));
        assert_eq!(loap.task.as_deref(), Some("block-a"));
        assert_eq!((loap.mem, loap.processes), ((300 + 120 + 10 + 60 + 500) * MB, 5));
        let kids: Vec<_> = loap.children.iter().map(|c| (c.label.as_str(), c.mem, c.processes)).collect();
        assert_eq!(
            kids,
            [
                ("CLI", 420 * MB, 2),
                ("Helpers (MCP servers, shell hooks)", 10 * MB, 1),
                ("Tools it started", 560 * MB, 2),
            ]
        );
    }

    #[test]
    fn an_agent_with_no_helpers_has_no_helpers_line() {
        let t = task(
            "a",
            TowerTaskKind::Agent,
            "Solo",
            vec![proc("codex.exe", Some(MB), Some(TowerProcessRole::Main), None)],
        );
        let m = build(&snapshot(vec![t]));
        let kids: Vec<_> = part(&m, TowerMemoryPartId::Agents).rows[0].children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(kids, ["CLI"]);
    }

    #[test]
    fn a_process_whose_memory_could_not_be_read_is_counted_but_adds_nothing() {
        let t = task(
            "agentmux",
            TowerTaskKind::Agentmux,
            "AgentMux",
            vec![
                proc("agentmux-srv.exe", Some(10 * MB), None, Some("Server")),
                proc("elevated.exe", None, None, None),
            ],
        );
        let m = build(&snapshot(vec![t]));
        assert_eq!((m.total, m.processes, m.unreadable), (10 * MB, 2, 1));
    }

    #[test]
    fn exited_processes_are_not_in_the_total() {
        let mut t = task(
            "block-a",
            TowerTaskKind::Agent,
            "Loap",
            vec![proc("claude.exe", Some(MB), Some(TowerProcessRole::Main), None)],
        );
        t.exited = Some(vec![proc("node.exe", None, Some(TowerProcessRole::Started), None)]);
        let m = build(&snapshot(vec![t]));
        assert_eq!((m.total, m.processes, m.unreadable), (MB, 1, 0));
    }

    #[test]
    fn another_instance_is_beside_the_total_not_in_it() {
        let mut snap = fixture();
        snap.machine = Some(TowerMachine {
            cpu: None,
            mem: 0,
            processes: 0,
            mem_total: None,
            agentmux_elsewhere: Some(1000 * MB),
        });
        let with = build(&snap);
        let without = build(&fixture());
        assert_eq!(with.other_instances, Some(1000 * MB));
        assert_eq!(with.total, without.total);
        assert_eq!(without.other_instances, None);
    }

    #[test]
    fn keys_are_the_same_from_one_sample_to_the_next_whatever_the_order() {
        let a = build(&fixture());
        let mut snap = fixture();
        snap.tasks.reverse();
        for t in &mut snap.tasks {
            t.processes.reverse();
        }
        let b = build(&snap);
        let keys = |m: &TowerMemory| -> Vec<Vec<String>> {
            m.parts.iter().map(|p| p.rows.iter().map(|r| r.key.clone()).collect()).collect()
        };
        assert_eq!(keys(&a), keys(&b));
    }
}
