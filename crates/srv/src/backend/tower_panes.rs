// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower's Panes view: this machine's AgentMux grouped by window, tab and pane,
//! each pane with what it costs (SPEC_TOWER_PANES_VIEW_2026_10_21.md).
//!
//! Built from a Tower sample, the store's layout and the window host's renderer
//! map, so it measures exactly what the other views do:
//! - an agent or terminal pane: its task (the processes it started);
//! - a browser pane: the renderers its page runs in, its own and each
//!   cross-site frame's (`crates/cef/src/renderer_map.rs`). A renderer serving
//!   several panes is shared: listed under each, counted once;
//! - every other pane is drawn in its window's renderer: it has no measured
//!   number of its own, and the window's row carries that renderer.

use std::collections::{BTreeSet, HashMap};

use crate::backend::obj::{self, Block, Tab, Window, Workspace};
use crate::backend::rpc_types::{
    TowerPane, TowerPaneTab, TowerPaneWindow, TowerPanes, TowerProcess, TowerSnapshot, TowerTaskKind,
};
use crate::backend::storage::store::Store;
use crate::backend::tower_agentmux::{page_host, RendererEntry};

/// Block meta while a browser pane is hibernated (browser-pane hibernation, PR 5290).
const SLEEP_META_KEY: &str = "browser:sleep";

/// A pane as the layout has it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPane {
    pub block_id: String,
    pub view: String,
    pub title: String,
    pub asleep: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTab {
    pub tab_id: String,
    pub name: String,
    pub shown: bool,
    pub panes: Vec<LayoutPane>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutWindow {
    pub window_id: String,
    pub name: String,
    pub tabs: Vec<LayoutTab>,
}

/// Every window, its tabs in order (pinned first) and their panes, from the store.
pub fn read_layout(store: &Store) -> Vec<LayoutWindow> {
    let workspaces = store.get_all::<Workspace>().unwrap_or_default();
    let tabs = store.get_all::<Tab>().unwrap_or_default();
    let blocks: HashMap<String, Block> =
        store.get_all::<Block>().unwrap_or_default().into_iter().map(|b| (b.oid.clone(), b)).collect();
    store
        .get_all::<Window>()
        .unwrap_or_default()
        .into_iter()
        .map(|w| {
            let ws = workspaces.iter().find(|x| x.oid == w.workspaceid);
            let display = obj::meta_get_string(&w.meta, "window:displayname", "");
            let name = if !display.is_empty() {
                display
            } else {
                ws.map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or_default()
            };
            let tabs = ws
                .map(|ws| {
                    ws.pinnedtabids
                        .iter()
                        .chain(ws.tabids.iter())
                        .filter_map(|tid| tabs.iter().find(|t| &t.oid == tid))
                        .map(|t| LayoutTab {
                            tab_id: t.oid.clone(),
                            name: t.name.clone(),
                            shown: ws.activetabid == t.oid,
                            panes: t.blockids.iter().filter_map(|bid| blocks.get(bid)).map(layout_pane).collect(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            LayoutWindow { window_id: w.oid.clone(), name, tabs }
        })
        .collect()
}

fn layout_pane(b: &Block) -> LayoutPane {
    let meta = |key: &str| obj::meta_get_string(&b.meta, key, "");
    let view = meta("view");
    let title = [meta("frame:title"), meta("agentName")]
        .into_iter()
        .find(|t| !t.is_empty())
        .or_else(|| (view == "browser").then(|| page_host(&meta("url"))).flatten())
        .unwrap_or_default();
    LayoutPane {
        block_id: b.oid.clone(),
        view,
        title,
        asleep: b.meta.get(SLEEP_META_KEY).is_some_and(|v| v.is_object()),
    }
}

fn sum_cpu(procs: &[TowerProcess]) -> Option<f64> {
    let measured: Vec<f64> = procs.iter().filter_map(|p| p.cpu).collect();
    (!measured.is_empty()).then(|| measured.iter().sum())
}

fn sum_mem(procs: &[TowerProcess]) -> u64 {
    procs.iter().filter_map(|p| p.mem).sum()
}

/// The Panes view from a sample. `view_pid` maps a process's PID to the PID its
/// renderer reports for itself (they differ only inside Linux's sandbox).
pub fn build(
    layout: &[LayoutWindow],
    snap: &TowerSnapshot,
    renderers: &[RendererEntry],
    view_pid: &dyn Fn(u32) -> u32,
) -> TowerPanes {
    // AgentMux's own processes, by the PID a renderer reports.
    let own: HashMap<u32, &TowerProcess> = snap
        .tasks
        .iter()
        .filter(|t| t.kind == TowerTaskKind::Agentmux)
        .flat_map(|t| t.processes.iter())
        .map(|p| (view_pid(p.pid), p))
        .collect();
    let procs_of = |pids: &BTreeSet<u32>| -> Vec<TowerProcess> {
        pids.iter().filter_map(|pid| own.get(pid).map(|p| (*p).clone())).collect()
    };

    // Which browser panes, windows and pool windows each renderer serves.
    let mut pane_pids: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    let mut panes_of_pid: HashMap<u32, BTreeSet<&str>> = HashMap::new();
    let mut window_pids: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    let mut pool_pids: BTreeSet<u32> = BTreeSet::new();
    for e in renderers {
        if let Some(block) = e.block_id.as_deref() {
            pane_pids.entry(block).or_default().insert(e.pid);
            panes_of_pid.entry(e.pid).or_default().insert(block);
        } else if let Some(window) = e.window_id.as_deref() {
            window_pids.entry(window).or_default().insert(e.pid);
        } else if e.pool {
            pool_pids.insert(e.pid);
        }
    }

    let mut shared_pids: BTreeSet<u32> = BTreeSet::new();
    let windows = layout
        .iter()
        .map(|w| {
            let tabs = w
                .tabs
                .iter()
                .map(|t| TowerPaneTab {
                    tab_id: t.tab_id.clone(),
                    name: t.name.clone(),
                    shown: t.shown,
                    panes: t
                        .panes
                        .iter()
                        .map(|p| {
                            let state = if p.asleep {
                                "asleep"
                            } else if t.shown {
                                "shown"
                            } else {
                                "hidden"
                            };
                            let mut pane = TowerPane {
                                block_id: p.block_id.clone(),
                                view: p.view.clone(),
                                title: p.title.clone(),
                                state: state.to_string(),
                                task: None,
                                cpu: None,
                                mem: 0,
                                processes: Vec::new(),
                                shared: Vec::new(),
                            };
                            if let Some(task) = snap.tasks.iter().find(|x| x.id == p.block_id) {
                                pane.task = Some(task.id.clone());
                                pane.cpu = task.cpu;
                                pane.mem = task.mem;
                            } else if let Some(pids) = pane_pids.get(p.block_id.as_str()) {
                                let (alone, shared): (BTreeSet<u32>, BTreeSet<u32>) =
                                    pids.iter().partition(|pid| panes_of_pid.get(*pid).is_none_or(|s| s.len() < 2));
                                shared_pids.extend(shared.iter().copied());
                                pane.processes = procs_of(&alone);
                                pane.shared = procs_of(&shared);
                                pane.cpu = sum_cpu(&pane.processes);
                                pane.mem = sum_mem(&pane.processes);
                            }
                            pane
                        })
                        .collect(),
                })
                .collect();
            let renderers = window_pids.get(w.window_id.as_str()).map(procs_of).unwrap_or_default();
            TowerPaneWindow {
                window_id: w.window_id.clone(),
                name: w.name.clone(),
                cpu: sum_cpu(&renderers),
                mem: sum_mem(&renderers),
                renderers,
                tabs,
            }
        })
        .collect();
    TowerPanes { windows, pool: procs_of(&pool_pids), shared_mem: sum_mem(&procs_of(&shared_pids)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::TowerTask;

    fn proc(pid: u32, mem: u64, cpu: f64) -> TowerProcess {
        TowerProcess { id: format!("{pid}:1"), pid, name: "agentmux".into(), mem: Some(mem), cpu: Some(cpu), ..Default::default() }
    }

    fn task(id: &str, kind: TowerTaskKind, mem: u64, processes: Vec<TowerProcess>) -> TowerTask {
        TowerTask {
            id: id.into(),
            kind,
            label: id.into(),
            tracking: "high".into(),
            cpu: Some(0.5),
            cpu_account: false,
            mem,
            processes,
            exited: None,
        }
    }

    fn snapshot(tasks: Vec<TowerTask>) -> TowerSnapshot {
        TowerSnapshot {
            ts_ms: 0,
            hostname: "h".into(),
            panes: None,
            os: "windows".into(),
            cpu_count: 8,
            memory_metric: "private working set".into(),
            interval_ms: 2000,
            remote: false,
            tasks,
            machine: None,
            host: None,
        }
    }

    fn entry(pid: u32, label: &str) -> RendererEntry {
        RendererEntry { pid, label: label.into(), main: true, block_id: None, pool: false, window_id: None }
    }

    fn pane(id: &str, view: &str) -> LayoutPane {
        LayoutPane { block_id: id.into(), view: view.into(), title: String::new(), asleep: false }
    }

    fn layout() -> Vec<LayoutWindow> {
        vec![LayoutWindow {
            window_id: "w1".into(),
            name: "Main".into(),
            tabs: vec![
                LayoutTab {
                    tab_id: "t1".into(),
                    name: "Tab 1".into(),
                    shown: true,
                    panes: vec![pane("agent-1", "agent"), pane("gh", "browser"), pane("swarm-1", "swarm")],
                },
                LayoutTab {
                    tab_id: "t2".into(),
                    name: "Tab 2".into(),
                    shown: false,
                    panes: vec![
                        pane("news", "browser"),
                        pane("wiki", "browser"),
                        LayoutPane { asleep: true, ..pane("slept", "browser") },
                    ],
                },
            ],
        }]
    }

    fn sample() -> (TowerSnapshot, Vec<RendererEntry>) {
        let snap = snapshot(vec![
            task("agent-1", TowerTaskKind::Agent, 300, vec![proc(500, 300, 0.5)]),
            task(
                "agentmux",
                TowerTaskKind::Agentmux,
                0,
                vec![proc(10, 2000, 0.3), proc(20, 180, 0.1), proc(21, 60, 0.0), proc(30, 90, 0.0), proc(40, 250, 0.0)],
            ),
        ]);
        let renderers = vec![
            RendererEntry { window_id: Some("w1".into()), ..entry(10, "main") },
            RendererEntry { block_id: Some("gh".into()), ..entry(20, "pane-gh") },
            RendererEntry { block_id: Some("gh".into()), main: false, ..entry(21, "pane-gh") },
            // Chromium put two pages of one site in one process.
            RendererEntry { block_id: Some("news".into()), ..entry(30, "pane-news") },
            RendererEntry { block_id: Some("wiki".into()), ..entry(30, "pane-wiki") },
            RendererEntry { pool: true, ..entry(40, "window-pool-1") },
        ];
        (snap, renderers)
    }

    #[test]
    fn panes_are_grouped_by_window_and_tab_with_their_state() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let w = &v.windows[0];
        assert_eq!(w.name, "Main");
        let states: Vec<(&str, &str)> =
            w.tabs.iter().flat_map(|t| t.panes.iter()).map(|p| (p.block_id.as_str(), p.state.as_str())).collect();
        assert_eq!(
            states,
            vec![
                ("agent-1", "shown"),
                ("gh", "shown"),
                ("swarm-1", "shown"),
                ("news", "hidden"),
                ("wiki", "hidden"),
                ("slept", "asleep")
            ]
        );
        assert!(w.tabs[0].shown && !w.tabs[1].shown);
    }

    #[test]
    fn an_agent_pane_costs_its_task() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let agent = &v.windows[0].tabs[0].panes[0];
        assert_eq!(agent.task.as_deref(), Some("agent-1"));
        assert_eq!(agent.mem, 300);
        assert_eq!(agent.cpu, Some(0.5));
    }

    #[test]
    fn a_browser_pane_costs_its_own_renderer_and_its_frames() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let gh = &v.windows[0].tabs[0].panes[1];
        assert_eq!(gh.processes.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![20, 21]);
        assert_eq!(gh.mem, 240);
        assert!(gh.shared.is_empty());
    }

    #[test]
    fn a_shared_renderer_is_listed_under_each_pane_and_counted_once() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let tab2 = &v.windows[0].tabs[1];
        for p in &tab2.panes[..2] {
            assert_eq!(p.mem, 0, "{} counts no shared memory", p.block_id);
            assert_eq!(p.shared.iter().map(|x| x.pid).collect::<Vec<_>>(), vec![30]);
        }
        assert_eq!(v.shared_mem, 90);
    }

    #[test]
    fn panes_drawn_in_the_window_have_no_number_and_the_window_carries_its_renderer() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let swarm = &v.windows[0].tabs[0].panes[2];
        assert_eq!((swarm.mem, swarm.cpu, swarm.processes.len()), (0, None, 0));
        assert_eq!(v.windows[0].mem, 2000);
        assert_eq!(v.pool.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![40]);
    }

    #[test]
    fn a_sleeping_pane_has_no_renderer_left() {
        let (snap, renderers) = sample();
        let v = build(&layout(), &snap, &renderers, &|p| p);
        let slept = &v.windows[0].tabs[1].panes[2];
        assert_eq!((slept.state.as_str(), slept.mem), ("asleep", 0));
    }

    #[test]
    fn renderers_are_matched_by_the_pid_they_report_for_themselves() {
        let (snap, renderers) = sample();
        // Inside Linux's sandbox a renderer reports a namespace PID (here, host PID + 1000).
        let shifted: Vec<RendererEntry> = renderers.into_iter().map(|e| RendererEntry { pid: e.pid + 1000, ..e }).collect();
        let v = build(&layout(), &snap, &shifted, &|p| p + 1000);
        assert_eq!(v.windows[0].tabs[0].panes[1].mem, 240);
    }
}
