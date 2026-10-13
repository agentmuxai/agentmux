// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The panes a paired device lists (`GET /agentmux/viewer/panes`): every pane
//! of every window and tab, in the order the desktop shows them, plus each
//! agent's shell drawer (a headless terminal named by the agent block's
//! `term:shellsubblockid`, not in any tab). Names, types and whether they
//! run; never a command line, a working directory or any content.
//!
//! An agent hidden from paired devices is left out, and so is its shell
//! drawer. Pure over the object tree and the lookups in [`PaneFacts`], so it
//! is tested without controllers or a registry.

use serde::Serialize;

use crate::backend::obj::{self, Block, Tab, Window, Workspace};

/// The meta key on an agent block naming its shell drawer's block.
pub const META_SHELL_SUBBLOCK_ID: &str = "term:shellsubblockid";

/// What a device can do with a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneKind {
    /// An agent: its feed opens by `agent`.
    Agent,
    /// A terminal.
    Terminal,
    /// Any other pane (editor, browser, files, a widget, ...): listed, opened
    /// on the computer.
    Other,
}

/// One pane, as a device gets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneEntry {
    pub block_id: String,
    /// The desktop's view id (`agent`, `term`, `editor`, `ext:<id>`, ...).
    pub view: String,
    pub kind: PaneKind,
    pub label: String,
    pub window_id: String,
    pub window: String,
    pub tab_id: String,
    pub tab: String,
    pub tab_active: bool,
    /// For an agent: the name its feed route takes, while it is registered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// For a terminal: whether its shell is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    /// For a shell drawer: the agent pane it belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_block_id: Option<String>,
}

/// The live facts the object tree doesn't hold.
pub struct PaneFacts<'a> {
    /// The pane's label and whether it is an agent (Tower's `block_label`).
    pub label: &'a dyn Fn(&Block) -> (String, bool),
    /// The name a registered agent runs under in this block.
    pub agent_name: &'a dyn Fn(&str) -> Option<String>,
    /// Whether a terminal block's shell is running; `None` without a controller.
    pub running: &'a dyn Fn(&str) -> Option<bool>,
    /// Whether the agent in this block is hidden from paired devices.
    pub hidden: &'a dyn Fn(&str) -> bool,
}

/// Every pane, window by window, tab by tab (pinned tabs first, as shown), each
/// agent's shell drawer right after its agent.
pub fn list(windows: &[Window], workspaces: &[Workspace], tabs: &[Tab], blocks: &[Block], facts: &PaneFacts) -> Vec<PaneEntry> {
    let block = |id: &str| blocks.iter().find(|b| b.oid == id);
    let mut out = Vec::new();
    for window in windows {
        let Some(ws) = workspaces.iter().find(|w| w.oid == window.workspaceid) else { continue };
        let window_name = obj::meta_get_string(&window.meta, "window:displayname", "");
        for tab in ws.pinnedtabids.iter().chain(ws.tabids.iter()).filter_map(|id| tabs.iter().find(|t| &t.oid == id)) {
            let entry = |b: &Block, kind: PaneKind, label: String| PaneEntry {
                block_id: b.oid.clone(),
                view: obj::meta_get_string(&b.meta, "view", ""),
                kind,
                label,
                window_id: window.oid.clone(),
                window: window_name.clone(),
                tab_id: tab.oid.clone(),
                tab: tab.name.clone(),
                tab_active: ws.activetabid == tab.oid,
                agent: None,
                running: None,
                parent_block_id: None,
            };
            for b in tab.blockids.iter().filter_map(|id| block(id)) {
                let (label, agent) = (facts.label)(b);
                let view = obj::meta_get_string(&b.meta, "view", "");
                if agent || view == "agent" {
                    if (facts.hidden)(&b.oid) {
                        continue;
                    }
                    out.push(PaneEntry { agent: (facts.agent_name)(&b.oid), ..entry(b, PaneKind::Agent, label.clone()) });
                    let drawer = obj::meta_get_string(&b.meta, META_SHELL_SUBBLOCK_ID, "");
                    if let Some(shell) = block(&drawer).filter(|_| !drawer.is_empty()) {
                        out.push(PaneEntry {
                            running: (facts.running)(&shell.oid),
                            parent_block_id: Some(b.oid.clone()),
                            ..entry(shell, PaneKind::Terminal, format!("{label} shell"))
                        });
                    }
                } else if view == "term" {
                    out.push(PaneEntry { running: (facts.running)(&b.oid), ..entry(b, PaneKind::Terminal, label) });
                } else {
                    out.push(entry(b, PaneKind::Other, label));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, view: &str, extra: &[(&str, &str)]) -> Block {
        let mut b = Block { oid: id.into(), ..Default::default() };
        b.meta.insert("view".into(), serde_json::json!(view));
        for (k, v) in extra {
            b.meta.insert((*k).into(), serde_json::json!(v));
        }
        b
    }

    fn tab(id: &str, name: &str, blocks: &[&str]) -> Tab {
        Tab { oid: id.into(), name: name.into(), blockids: blocks.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    fn label(b: &Block) -> (String, bool) {
        let title = obj::meta_get_string(&b.meta, "frame:title", "");
        let agent = obj::meta_get_string(&b.meta, "agentName", "");
        if !agent.is_empty() {
            (agent, true)
        } else {
            (if title.is_empty() { "Terminal".into() } else { title }, false)
        }
    }

    fn tree() -> (Vec<Window>, Vec<Workspace>, Vec<Tab>, Vec<Block>) {
        let mut window = Window { oid: "w1".into(), workspaceid: "ws1".into(), ..Default::default() };
        window.meta.insert("window:displayname".into(), serde_json::json!("Main"));
        let ws = Workspace {
            oid: "ws1".into(),
            tabids: vec!["t2".into()],
            pinnedtabids: vec!["t1".into()],
            activetabid: "t2".into(),
            ..Default::default()
        };
        let tabs = vec![tab("t1", "Pinned", &["ed"]), tab("t2", "Work", &["ag", "hid", "sh", "gone"])];
        let blocks = vec![
            block("ed", "editor", &[("frame:title", "notes.md")]),
            block("ag", "agent", &[("agentName", "Ada"), (META_SHELL_SUBBLOCK_ID, "drawer")]),
            block("drawer", "term", &[]),
            block("hid", "agent", &[("agentName", "Secret"), (META_SHELL_SUBBLOCK_ID, "hid-drawer")]),
            block("hid-drawer", "term", &[]),
            block("sh", "term", &[("frame:title", "build")]),
        ];
        (vec![window], vec![ws], tabs, blocks)
    }

    #[test]
    fn every_pane_in_order_with_drawers_after_their_agents_and_hidden_agents_left_out() {
        let (windows, workspaces, tabs, blocks) = tree();
        let facts = PaneFacts {
            label: &label,
            agent_name: &|id| (id == "ag").then(|| "ada".to_string()),
            running: &|id| match id {
                "drawer" => Some(true),
                "sh" => Some(false),
                _ => None,
            },
            hidden: &|id| id == "hid",
        };
        let panes = list(&windows, &workspaces, &tabs, &blocks, &facts);
        let ids: Vec<&str> = panes.iter().map(|p| p.block_id.as_str()).collect();
        assert_eq!(ids, ["ed", "ag", "drawer", "sh"], "pinned tab first; a missing block is skipped");

        assert_eq!(panes[0].kind, PaneKind::Other);
        assert_eq!((panes[0].label.as_str(), panes[0].tab.as_str(), panes[0].tab_active), ("notes.md", "Pinned", false));
        assert_eq!(panes[0].window, "Main");

        assert_eq!(panes[1].kind, PaneKind::Agent);
        assert_eq!(panes[1].agent.as_deref(), Some("ada"));
        assert!(panes[1].tab_active);

        assert_eq!(panes[2].kind, PaneKind::Terminal);
        assert_eq!(panes[2].label, "Ada shell");
        assert_eq!(panes[2].parent_block_id.as_deref(), Some("ag"));
        assert_eq!(panes[2].running, Some(true));
        assert_eq!(panes[2].tab_id, "t2", "the drawer is shown where its agent is");

        assert_eq!((panes[3].kind, panes[3].running), (PaneKind::Terminal, Some(false)));
        assert!(!ids.contains(&"hid-drawer"), "a hidden agent's shell is hidden with it");
    }

    #[test]
    fn the_wire_omits_what_a_pane_does_not_have() {
        let (windows, workspaces, tabs, blocks) = tree();
        let facts = PaneFacts { label: &label, agent_name: &|_| None, running: &|_| None, hidden: &|_| false };
        let panes = list(&windows, &workspaces, &tabs, &blocks, &facts);
        let editor = serde_json::to_value(&panes[0]).unwrap();
        assert_eq!(editor["kind"], "other");
        assert_eq!(editor["view"], "editor");
        for absent in ["agent", "running", "parent_block_id"] {
            assert!(editor.get(absent).is_none(), "{absent}");
        }
        let agent = serde_json::to_value(panes.iter().find(|p| p.block_id == "ag").unwrap()).unwrap();
        assert_eq!(agent["kind"], "agent");
        assert!(agent.get("agent").is_none(), "an agent that is not registered has no feed name");
    }
}
