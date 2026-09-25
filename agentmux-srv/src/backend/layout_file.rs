// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout files — the `agentmux.layout` v1 format and its writer
//! (`docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md`, Phase 1).
//!
//! A layout file describes a window's tabs, their split trees (as ratios)
//! and each pane's typed views, so it can be read, diffed, shared and — in
//! Phase 2 — reopened. It is **not** a dump of the store: every view is
//! written through a per-type allowlist ([`view_from_block`]), never by
//! copying block meta. The session-restore snapshot
//! (`server/service/session_restore.rs`) copies meta verbatim, which is fine
//! for an internal blob and wrong for a file a user hands to someone else:
//! session ids, instance ids, absolute paths and env would all leak.
//!
//! Every struct carries an `extra` map, so a reader keeps fields it doesn't
//! know and writes them back unchanged (spec §3.1).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use agentmux_common::layout_types::LayoutNode;

use crate::backend::obj::{Block, LayoutState, MetaMapType, Tab, Window, Workspace};
use crate::backend::storage::store::Store;

/// The `format` discriminator. A reader refuses any other value.
pub const LAYOUT_FORMAT: &str = "agentmux.layout";
/// The format version this build writes and the newest it reads.
pub const LAYOUT_VERSION: u32 = 1;
/// Every layout file ends with this (spec §3.1): filterable in dialogs,
/// highlighted as JSON in any editor.
pub const LAYOUT_EXTENSION: &str = ".agentmux-layout.json";

// ── The file format (spec §3) ──

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutDoc {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub saved_at: String,
    #[serde(default)]
    pub saved_by: SavedBy,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub windows: Vec<LayoutWindow>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedBy {
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub app_version: String,
    /// This install's WAN instance id (public), so Phase 2 can tell "saved
    /// here" from "came from elsewhere". Absent without a `wan.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<usize>,
    #[serde(default)]
    pub tabs: Vec<LayoutTab>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutTab {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<TreeNode>,
    /// Keyed by file-local pane id (`p1`, `p2`, …) — never an OID.
    #[serde(default)]
    pub panes: BTreeMap<String, LayoutPane>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A split (`{split, children}`) or a leaf (`{pane}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TreeNode {
    Split {
        /// `row` | `column` — the tree's own `flexDirection` values.
        split: String,
        children: Vec<TreeChild>,
        #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
        extra: Map<String, Value>,
    },
    Leaf {
        pane: String,
        #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
        extra: Map<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeChild {
    /// Share of the parent split; siblings sum to 1.
    pub ratio: f64,
    pub node: TreeNode,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutPane {
    /// The pane's tabs, in order.
    pub views: Vec<LayoutView>,
    /// Index into `views`.
    #[serde(default)]
    pub active: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized: Option<bool>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutView {
    #[serde(rename = "type")]
    pub view_type: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub config: Map<String, Value>,
    /// Opt-in, machine-local resume references (spec §3.3). Absent unless the
    /// user asked for them at save time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Map<String, Value>>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

// ── Export ──

/// Everything the exporter reads from outside the store, injected so tests
/// are deterministic.
pub struct ExportContext {
    /// The user's home dir; paths under it are written `~/…`.
    pub home: Option<PathBuf>,
    pub app_version: String,
    pub instance: Option<String>,
    /// RFC 3339.
    pub saved_at: String,
    /// Write the opt-in `resume` references (spec §3.3; off by default).
    pub include_resume: bool,
}

impl ExportContext {
    /// The live context: real home dir, this build's version, this install's
    /// WAN instance id if it has one, now.
    pub fn live(include_resume: bool) -> Self {
        Self {
            home: dirs::home_dir(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            instance: crate::backend::storage::wan_identity::global()
                .and_then(|w| w.instance_ensure(&crate::backend::reactive::registry::local_host_label()).ok())
                .map(|i| i.instance_id),
            saved_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            include_resume,
        }
    }
}

/// The result of exporting: the document, plus anything the user should be
/// told before it is written (spec §3.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub doc: LayoutDoc,
    pub warnings: Vec<String>,
}

/// Export one window — every tab of its workspace — as a layout document.
pub fn export_window(store: &Store, window_id: &str, name: &str, ctx: &ExportContext) -> Result<Exported, String> {
    let window = store
        .get::<Window>(window_id)
        .map_err(|e| format!("could not read window {window_id}: {e}"))?
        .ok_or_else(|| format!("no such window: {window_id}"))?;
    let workspace = store
        .get::<Workspace>(&window.workspaceid)
        .map_err(|e| format!("could not read the window's workspace: {e}"))?
        .ok_or_else(|| "the window has no workspace".to_string())?;

    let mut warnings = Vec::new();
    let mut tabs = Vec::new();
    let mut active_tab = None;
    // `pinnedtabids` is legacy but can still hold a fresh workspace's only
    // tab — read both, as `session_restore::snapshot_workspace` does.
    for tab_id in workspace.pinnedtabids.iter().chain(workspace.tabids.iter()) {
        let Some(tab) = store.get::<Tab>(tab_id).ok().flatten() else { continue };
        let Some(exported) = export_tab(store, &tab, ctx, &mut warnings) else { continue };
        if tab_id == &workspace.activetabid {
            active_tab = Some(tabs.len());
        }
        tabs.push(exported);
    }
    if tabs.is_empty() {
        return Err("the window has no tabs with panes to save".to_string());
    }
    Ok(Exported {
        doc: LayoutDoc {
            format: LAYOUT_FORMAT.to_string(),
            version: LAYOUT_VERSION,
            name: name.to_string(),
            saved_at: ctx.saved_at.clone(),
            saved_by: SavedBy {
                app: "agentmux".to_string(),
                app_version: ctx.app_version.clone(),
                instance: ctx.instance.clone(),
                extra: Map::new(),
            },
            scope: "window".to_string(),
            windows: vec![LayoutWindow { active_tab, tabs, extra: Map::new() }],
            extra: Map::new(),
        },
        warnings,
    })
}

/// Pane ids are minted per tab in tree order; node ids are the tree's own,
/// used only to map focus/magnify onto them.
struct TabBuilder<'a> {
    store: &'a Store,
    ctx: &'a ExportContext,
    blocks: HashMap<String, Block>,
    panes: BTreeMap<String, LayoutPane>,
    pane_by_node: HashMap<String, String>,
    warnings: &'a mut Vec<String>,
}

fn export_tab(store: &Store, tab: &Tab, ctx: &ExportContext, warnings: &mut Vec<String>) -> Option<LayoutTab> {
    let layout = if tab.layoutstate.is_empty() {
        None
    } else {
        store.get::<LayoutState>(&tab.layoutstate).ok().flatten()
    };
    let rootnode = layout.as_ref().and_then(|l| l.rootnode.clone())?;
    let blocks = tab
        .blockids
        .iter()
        .filter_map(|id| store.get::<Block>(id).ok().flatten().map(|b| (id.clone(), b)))
        .collect();
    let mut builder = TabBuilder {
        store,
        ctx,
        blocks,
        panes: BTreeMap::new(),
        pane_by_node: HashMap::new(),
        warnings,
    };
    let root = builder.node(&rootnode)?;
    let layout = layout?;
    let focused = builder.pane_by_node.get(&layout.focusednodeid).cloned();
    let magnified = builder.pane_by_node.get(&layout.magnifiednodeid).cloned();
    Some(LayoutTab {
        name: tab.name.clone(),
        focused,
        magnified,
        root: Some(root),
        panes: builder.panes,
        extra: Map::new(),
    })
}

impl TabBuilder<'_> {
    /// Convert one tree node; `None` if nothing under it resolves to a view
    /// (a pane whose blocks are gone is dropped, not written empty).
    fn node(&mut self, node: &LayoutNode) -> Option<TreeNode> {
        if node.children.is_empty() {
            return self.leaf(node);
        }
        let mut kept: Vec<(f64, TreeNode)> = Vec::new();
        for child in &node.children {
            if let Some(converted) = self.node(child) {
                kept.push((f64::from(child.size.max(0.0)), converted));
            }
        }
        match kept.len() {
            0 => None,
            // A split with one survivor is just that survivor.
            1 => kept.pop().map(|(_, n)| n),
            _ => Some(TreeNode::Split {
                split: match node.flex_direction {
                    agentmux_common::layout_types::FlexDirection::Column => "column",
                    _ => "row",
                }
                .to_string(),
                children: ratios(&kept.iter().map(|(s, _)| *s).collect::<Vec<_>>())
                    .into_iter()
                    .zip(kept.into_iter().map(|(_, n)| n))
                    .map(|(ratio, node)| TreeChild { ratio, node })
                    .collect(),
                extra: Map::new(),
            }),
        }
    }

    fn leaf(&mut self, node: &LayoutNode) -> Option<TreeNode> {
        let data = node.data.as_ref()?;
        let stack: Vec<&String> = if data.block_stack.is_empty() {
            vec![&data.block_id]
        } else {
            data.block_stack.iter().collect()
        };
        let active_block = if data.active_block_id.is_empty() { &data.block_id } else { &data.active_block_id };
        let mut views = Vec::new();
        let mut active = 0;
        for block_id in stack {
            let Some(block) = self.blocks.get(block_id) else { continue };
            if block_id == active_block {
                active = views.len();
            }
            views.push(view_from_block(self.store, &block.meta, self.ctx, self.warnings));
        }
        if views.is_empty() {
            return None;
        }
        let pane_id = format!("p{}", self.panes.len() + 1);
        self.pane_by_node.insert(node.id.clone(), pane_id.clone());
        self.panes.insert(
            pane_id.clone(),
            LayoutPane {
                views,
                active,
                minimized: node.extra.get("minimized").and_then(Value::as_bool).filter(|m| *m),
                extra: Map::new(),
            },
        );
        Some(TreeNode::Leaf { pane: pane_id, extra: Map::new() })
    }
}

/// Flex sizes → ratios summing to exactly 1, rounded to 4 places for a
/// readable file (the last sibling absorbs the rounding). Non-positive sizes
/// share equally.
fn ratios(sizes: &[f64]) -> Vec<f64> {
    let total: f64 = sizes.iter().sum();
    let n = sizes.len();
    let raw: Vec<f64> = if total > 0.0 {
        sizes.iter().map(|s| s / total).collect()
    } else {
        vec![1.0 / n as f64; n]
    };
    let mut out: Vec<f64> = raw.iter().map(|r| (r * 10_000.0).round() / 10_000.0).collect();
    if let Some(last) = out.last_mut() {
        let rest: f64 = raw.iter().take(n - 1).map(|r| (r * 10_000.0).round() / 10_000.0).sum();
        *last = ((1.0 - rest) * 10_000.0).round() / 10_000.0;
    }
    out
}

fn meta_str<'a>(meta: &'a MetaMapType, key: &str) -> Option<&'a str> {
    meta.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A path under `home` as `~/…` (forward slashes); anything else unchanged.
fn home_relative(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home.and_then(|h| h.to_str()) else { return path.to_string() };
    let home = home.trim_end_matches(['/', '\\']);
    // `get`, not `[..]`: the home dir's byte length can fall inside a
    // multi-byte character of `path`, and slicing there panics (ReAgent P1 on
    // #3787). Such a path can't be under home anyway.
    let (Some(prefix), Some(rest)) = (path.get(..home.len()), path.get(home.len()..)) else {
        return path.to_string();
    };
    let matches_prefix = if cfg!(windows) { prefix.eq_ignore_ascii_case(home) } else { prefix == home };
    if !matches_prefix {
        return path.to_string();
    }
    if rest.is_empty() {
        return "~".to_string();
    }
    if !rest.starts_with(['/', '\\']) {
        return path.to_string(); // `/home/alice2` is not under `/home/alice`
    }
    format!("~{}", rest.replace('\\', "/"))
}

/// A URL with its query string and fragment removed (spec §3.3/§3.4) —
/// they are where tokens, session ids and tracking live.
fn strip_query(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or("").to_string()
}

/// One pane tab, written from the per-type allowlist (spec §3.3). Anything
/// not named here — session ids, instance ids, env, runtime state, CLI
/// paths, failure records — is never written. An unknown view type is
/// written as `{type}` alone.
fn view_from_block(store: &Store, meta: &MetaMapType, ctx: &ExportContext, warnings: &mut Vec<String>) -> LayoutView {
    let view_type = meta_str(meta, "view").unwrap_or("unknown").to_string();
    let home = ctx.home.as_deref();
    let mut config = Map::new();
    let mut resume = None;
    match view_type.as_str() {
        "agent" => {
            // The definition's portable identity, never its local row id.
            let def = meta_str(meta, "agentId").and_then(|id| store.agent_def_get(id).ok().flatten());
            let slug = def.as_ref().map(|d| d.slug.clone()).filter(|s| !s.is_empty());
            let name = def
                .as_ref()
                .map(|d| d.name.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| meta_str(meta, "agentName").map(str::to_string));
            let provider = def
                .as_ref()
                .map(|d| d.provider.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| meta_str(meta, "agentProvider").map(str::to_string));
            if let Some(slug) = slug {
                config.insert("agent".into(), Value::String(slug));
            }
            if let Some(name) = name {
                config.insert("name".into(), Value::String(name));
            }
            if let Some(provider) = provider {
                config.insert("provider".into(), Value::String(provider));
            }
            if ctx.include_resume {
                if let Some(session) = meta_str(meta, "agent:sessionid") {
                    let mut r = Map::new();
                    r.insert("session_id".into(), Value::String(session.to_string()));
                    resume = Some(r);
                }
            }
        }
        "term" => {
            if let Some(cwd) = meta_str(meta, "cmd:cwd") {
                config.insert("cwd".into(), Value::String(home_relative(cwd, home)));
            }
            if let Some(cmd) = meta_str(meta, "cmd") {
                let args: Vec<String> = meta
                    .get("cmd:args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let full = std::iter::once(cmd.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
                if crate::backend::reactive::sanitize::is_sensitive_message(&full) {
                    warnings.push(format!(
                        "A terminal command looks like it may contain a credential or a destructive operation: `{}` — check the file before sharing it.",
                        crate::backend::reactive::sanitize::sanitize_message(&full)
                    ));
                }
                config.insert("command".into(), Value::String(cmd.to_string()));
                if !args.is_empty() {
                    config.insert("args".into(), Value::Array(args.into_iter().map(Value::String).collect()));
                }
            }
        }
        "browser" => {
            if let Some(url) = meta_str(meta, "url") {
                config.insert("url".into(), Value::String(strip_query(url)));
            }
        }
        "editor" | "codeeditor" | "preview" => {
            if let Some(file) = meta_str(meta, "file") {
                config.insert("file".into(), Value::String(home_relative(file, home)));
            }
        }
        "media" => {
            if let Some(path) = meta_str(meta, "media:path") {
                config.insert("path".into(), Value::String(home_relative(path, home)));
            }
        }
        "sysinfo" => {
            if let Some(kind) = meta_str(meta, "sysinfo:type") {
                config.insert("sysinfo_type".into(), Value::String(kind.to_string()));
            }
        }
        "armory" => {
            if let Some(section) = meta_str(meta, "armory:section") {
                config.insert("section".into(), Value::String(section.to_string()));
            }
        }
        // Singleton views and anything this build doesn't know: the type
        // alone is enough to reopen them, and nothing else is safe to assume.
        _ => {}
    }
    LayoutView { view_type, config, resume, extra: Map::new() }
}

// ── Write ──

/// Whether `path` is somewhere a layout may be written: absolute, and named
/// `*.agentmux-layout.json`.
pub fn is_layout_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(LAYOUT_EXTENSION) && n.len() > LAYOUT_EXTENSION.len())
}

/// Write `doc` to `path` atomically: a temp file beside it, then a rename,
/// so a crash or a full disk never leaves half a layout behind.
pub fn write_layout_file(path: &Path, doc: &LayoutDoc) -> Result<(), String> {
    if !is_layout_path(path) {
        return Err(format!(
            "a layout must be saved to an absolute path ending in {LAYOUT_EXTENSION}: {}",
            path.display()
        ));
    }
    let dir = path.parent().ok_or_else(|| format!("no parent directory: {}", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let mut json = serde_json::to_string_pretty(doc).map_err(|e| format!("could not encode the layout: {e}"))?;
    json.push('\n');
    let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("layout"), uuid::Uuid::new_v4()));
    std::fs::write(&tmp, json).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not save {}: {e}", path.display())
    })
}

/// Parse a layout document, refusing another format or a newer version
/// (spec §3.1). Unknown fields are kept in `extra`. Its production caller is
/// Phase 2's "Open layout…"; until then it pins the format in tests.
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_layout(json: &str) -> Result<LayoutDoc, String> {
    let doc: LayoutDoc = serde_json::from_str(json).map_err(|e| format!("not a layout file: {e}"))?;
    if doc.format != LAYOUT_FORMAT {
        return Err(format!("not an AgentMux layout (format is \"{}\")", doc.format));
    }
    if doc.version > LAYOUT_VERSION {
        return Err(format!(
            "this layout was saved by a newer AgentMux (format version {}; this build reads up to {LAYOUT_VERSION})",
            doc.version
        ));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests;
