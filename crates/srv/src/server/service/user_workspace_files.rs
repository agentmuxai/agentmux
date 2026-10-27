// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A workspace as a file (docs/specs/SPEC_WORKSPACES_2026_10_23.md §3.2, §4.2):
//! ☰ → Workspaces → Export… and Import….
//!
//! - **Export** writes the open workspace, every window of it with its name and
//!   whether it is closed, as an `agentmux.layout` document with
//!   `scope: "workspace"` (`layout_file::export_workspace`) to a
//!   `*.agentmux-workspace.json` file the host's Save dialog picked. The same
//!   per-view allowlist as a layout file: no session ids, and the
//!   machine-local `resume` references only when asked for.
//! - **Import** reads an exported workspace, or an older
//!   `*.agentmux-layout.json`, as a new workspace and switches to it. It is
//!   opened as a template, as Open layout… opened a layout: every pane is new,
//!   each agent pane starts its agent afresh, and a terminal's command runs
//!   only for a trusted file or when the user said so in the preview. The new
//!   workspace's record holds the windows to build; the switch builds them
//!   (`session_windows::restore_all`, as for a record whose contents are gone).

use std::path::{Path, PathBuf};

use agentmux_common::time::now_ms;
use serde_json::{json, Value};

use crate::backend::layout_file::{self, LayoutDoc, PlanOptions, WorkspacePlan, WorkspaceWindow};
use crate::backend::obj::{self, Block, Tab, Workspace};
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::app_api::agent_io::agent_panes_of;
use crate::server::AppState;

use super::user_workspace::current_id;
use super::user_workspace_switch::{insert_workspace, switch_to, Switched};

/// What decides whether a file is trusted (layout files spec §3.5): saved by
/// this install, in one of its own folders. Injected for tests.
pub(crate) struct ImportTrust {
    pub own_instance: Option<String>,
    pub dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
}

impl ImportTrust {
    fn live() -> Self {
        let shared = agentmux_common::DataPaths::from_env().map(|p| p.shared_dir);
        ImportTrust {
            own_instance: crate::backend::storage::wan_identity::global()
                .and_then(|w| w.instance_ensure(&crate::backend::reactive::registry::local_host_label()).ok())
                .map(|i| i.instance_id),
            dirs: shared.map(|s| vec![s.join("workspaces"), s.join("layouts")]).unwrap_or_default(),
            home: dirs::home_dir(),
        }
    }

    fn trusts(&self, doc: &LayoutDoc, path: &Path) -> bool {
        self.dirs.iter().any(|d| layout_file::is_trusted(doc, path, self.own_instance.as_deref(), Some(d)))
    }
}

/// Read a file Import… accepts: an exported workspace or a layout file.
fn read_doc(path: &str) -> Result<(LayoutDoc, PathBuf), String> {
    let path = PathBuf::from(path);
    if !layout_file::is_workspace_path(&path) && !layout_file::is_layout_path(&path) {
        return Err(format!(
            "the path must be absolute and end in {} or {}",
            layout_file::WORKSPACE_EXTENSION,
            layout_file::LAYOUT_EXTENSION
        ));
    }
    let doc = layout_file::read_layout_file(&path)?;
    Ok((doc, path))
}

/// The name an imported workspace gets: the file's, else its file name.
fn import_name(doc: &LayoutDoc, path: &Path) -> String {
    let named = doc.name.trim();
    if !named.is_empty() {
        return named.to_string();
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let lower = file.to_ascii_lowercase();
    let stem = [layout_file::WORKSPACE_EXTENSION, layout_file::LAYOUT_EXTENSION]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(file, |ext| &file[..file.len() - ext.len()])
        .trim();
    if stem.is_empty() { "Imported workspace".to_string() } else { stem.to_string() }
}

/// `name`, or `name 2`, `name 3`, … when a workspace has it already.
pub(crate) fn unique_name(store: &Store, name: &str) -> String {
    let taken: Vec<String> =
        store.user_workspace_list().unwrap_or_default().into_iter().map(|w| w.name.to_lowercase()).collect();
    if !taken.contains(&name.to_lowercase()) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|c| !taken.contains(&c.to_lowercase())).unwrap_or_default()
}

fn plan_of(store: &Store, doc: &LayoutDoc, trust: &ImportTrust, run_commands: bool) -> WorkspacePlan {
    layout_file::plan_workspace(store, doc, &PlanOptions { home: trust.home.clone(), run_commands })
}

/// What Import… shows before it opens a file: `{name, trusted, windows,
/// closed, tabs: [{name, panes}], commands, notes}`, the shape of a layout
/// preview plus the windows.
pub(crate) fn preview(store: &Store, path: &str, trust: &ImportTrust) -> Result<Value, String> {
    let (doc, path) = read_doc(path)?;
    let trusted = trust.trusts(&doc, &path);
    let plan = plan_of(store, &doc, trust, trusted);
    let tabs: Vec<Value> = plan
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter())
        .map(|t| json!({ "name": t.name, "panes": t.summary }))
        .collect();
    Ok(json!({
        "name": import_name(&doc, &path),
        "trusted": trusted,
        "windows": plan.windows.len(),
        "closed": plan.windows.iter().filter(|w| w.closed).count(),
        "tabs": tabs,
        "commands": plan.commands,
        "notes": plan.notes,
    }))
}

/// The record a new workspace starts with: one entry per planned window, in
/// the session record's shape without contents ids, so the switch builds each
/// one (`session_restore::restore_workspace`). A plan with no open window
/// opens its first.
pub(crate) fn import_record(plan: &WorkspacePlan) -> Option<Value> {
    let any_open = plan.windows.iter().any(|w| !w.closed);
    let now = now_ms();
    let entries: Vec<Value> = plan
        .windows
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let tabs: Vec<Value> = w
                .tabs
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "blocks": t.blocks.iter().map(|m| json!({ "meta": m })).collect::<Vec<_>>(),
                        "rootnode": t.rootnode.as_ref().and_then(|n| serde_json::to_value(n).ok()),
                        "focusednodeid": t.focused,
                        "magnifiednodeid": t.magnified,
                    })
                })
                .collect();
            let closed = w.closed && (any_open || i > 0);
            let mut entry = json!({
                "tabs": tabs,
                "state": if closed { layout_file::WINDOW_CLOSED } else { layout_file::WINDOW_OPEN },
                "window": w.name.as_ref().map_or_else(|| json!({}), |n| json!({ "name": n })),
            });
            if let Some(active) = w.active_tab {
                entry["active_tab_index"] = json!(active);
            }
            if closed {
                entry["closed_at_ms"] = json!(now);
            }
            entry
        })
        .collect();
    let first = entries.first()?.clone();
    Some(json!({ "tabs": first["tabs"], "windows": entries, "saved_at_ms": now }))
}

/// The agent panes a layout's plan made in contents `ws_id`, waiting for their
/// agent: `(tab, block, agent definition)`.
fn agent_placeholders(store: &Store, ws_id: &str) -> Vec<(String, String, String)> {
    let Some(ws) = store.get::<Workspace>(ws_id).ok().flatten() else { return Vec::new() };
    ws.pinnedtabids
        .iter()
        .chain(ws.tabids.iter())
        .filter_map(|t| store.get::<Tab>(t).ok().flatten())
        .flat_map(|tab| {
            tab.blockids
                .iter()
                .filter_map(|b| store.get::<Block>(b).ok().flatten())
                .filter_map(|b| {
                    let def = obj::meta_get_string(&b.meta, layout_file::META_LAYOUT_AGENT, "");
                    (!def.is_empty()).then(|| (tab.oid.clone(), b.oid.clone(), def))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Start each agent the shown windows' panes wait for (the window that asked,
/// and those waiting to open beside it). Returns a note per agent that didn't.
async fn start_agents(state: &AppState) -> Vec<String> {
    let store = &state.mstore;
    let mut shown = super::session_closed::open_workspaces(store);
    shown.extend(super::session_windows::waiting_workspaces(store));
    // The tabs the import is showing. A pane in a workspace that was closed
    // (the one this import replaced) is saved contents, not a second pane: it
    // only comes back by opening that workspace.
    let shown_tabs: std::collections::HashSet<String> = shown
        .iter()
        .filter_map(|ws| store.get::<Workspace>(ws).ok().flatten())
        .flat_map(|ws| ws.pinnedtabids.into_iter().chain(ws.tabids))
        .collect();
    let mut notes = Vec::new();
    for ws in shown {
        for (tab, block, def) in agent_placeholders(store, &ws) {
            let name = || {
                store
                    .get::<Block>(&block)
                    .ok()
                    .flatten()
                    .map(|b| obj::meta_get_string(&b.meta, "agentName", ""))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| def.clone())
            };
            // One pane per agent: an agent that already has a pane in the
            // windows being shown (any tab, running or not), such as the same
            // agent in two windows of the file, is not opened again here.
            let open_elsewhere: Vec<_> = agent_panes_of(store, &definition_id(store, &def))
                .into_iter()
                .filter(|pane| pane.block_id != block && shown_tabs.contains(&pane.tab_id))
                .collect();
            if let Some(pane) = open_elsewhere.first() {
                notes.push(format!(
                    "An agent ({}) already has a pane, in \"{}\", so its pane here shows the agent picker.",
                    name(),
                    pane.tab_name
                ));
                continue;
            }
            if let Err(e) = crate::server::app_api::agent_open::open_agent_into_block(state, &def, &tab, &block).await {
                notes.push(format!("An agent ({}) didn't start, so its pane shows the agent picker: {e}", name()));
            }
        }
    }
    notes
}

/// The id of the agent definition a layout names (its id, or a user agent's
/// name as `agent.open` reads it); what it names, as written, when none matches.
fn definition_id(store: &Store, wanted: &str) -> String {
    let agents = store.agent_def_list().unwrap_or_default();
    let mut users = agents.iter().filter(|a| a.is_seeded == 0);
    users
        .clone()
        .find(|a| a.id == wanted)
        .or_else(|| users.find(|a| a.name.eq_ignore_ascii_case(wanted)))
        .map_or_else(|| wanted.to_string(), |a| a.id.clone())
}

/// Import the file at `path` as a new workspace and switch to it from window
/// `window_id`. `run_commands` `None` means "as trusted". Returns the switch
/// and what the user should know.
pub(crate) async fn import(
    state: &AppState,
    path: &str,
    run_commands: Option<bool>,
    window_id: &str,
    trust: &ImportTrust,
) -> Result<(Switched, Vec<String>), String> {
    let store = &state.mstore;
    let (doc, file) = read_doc(path)?;
    let run = run_commands.unwrap_or_else(|| trust.trusts(&doc, &file));
    let plan = plan_of(store, &doc, trust, run);
    let record = import_record(&plan).ok_or_else(|| "the file has no windows that can be opened".to_string())?;
    let row = insert_workspace(store, &unique_name(store, &import_name(&doc, &file)))?;
    if let Err(e) = store.user_workspace_save_record(&row.id, &record.to_string(), now_ms()) {
        let _ = store.user_workspace_delete(&row.id);
        return Err(e.to_string());
    }
    let switched = match switch_to(state, &row.id, window_id).await {
        Ok(s) => s,
        Err(e) => {
            let _ = store.user_workspace_delete(&row.id);
            return Err(e);
        }
    };
    let mut notes = plan.notes;
    notes.extend(start_agents(state).await);
    if plan.windows.iter().any(|w| w.closed && w.tabs.iter().any(|t| t.blocks.iter().any(has_agent))) {
        notes.push("An agent pane in a closed window opens as the agent picker when you reopen it.".to_string());
    }
    tracing::info!(workspace = %switched.id, windows = plan.windows.len(), "user_workspace: imported {}", file.display());
    Ok((switched, notes))
}

fn has_agent(meta: &Value) -> bool {
    meta.get(layout_file::META_LAYOUT_AGENT).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())
}

/// Export the open workspace to `path` (`*.agentmux-workspace.json`). Returns
/// `{path, warnings}`: what the user should check before sharing it.
pub(crate) fn export(store: &Store, path: &str, ctx: &layout_file::ExportContext) -> Result<Value, String> {
    let path = PathBuf::from(path);
    if !layout_file::is_workspace_path(&path) {
        return Err(format!("the path must be absolute and end in {}", layout_file::WORKSPACE_EXTENSION));
    }
    let id = current_id(store).ok_or_else(|| "no workspace is open".to_string())?;
    let name = store.user_workspace_get(&id).ok().flatten().map(|w| w.name).unwrap_or_default();
    let mut windows: Vec<WorkspaceWindow> = super::session_windows::live_windows(store)
        .into_iter()
        .map(|(w, ws)| WorkspaceWindow {
            workspace_id: ws,
            name: super::session_windows::saved_name(store, &w),
            closed: false,
        })
        .collect();
    windows.extend(super::session_closed::closed_entries(store).iter().filter_map(|e| {
        Some(WorkspaceWindow {
            workspace_id: e.get("workspaceid")?.as_str()?.to_string(),
            name: e.get("window").and_then(|w| w.get("name")).and_then(|v| v.as_str()).map(str::to_string),
            closed: true,
        })
    }));
    let exported = layout_file::export_workspace(store, &name, &windows, ctx)?;
    layout_file::write_layout_file(&path, &exported.doc)?;
    Ok(json!({ "path": path.to_string_lossy(), "warnings": exported.warnings }))
}

// ---- Service handlers ----

/// `workspace.ExportUserWorkspace(path, includeResume)`.
pub(crate) async fn handle_export(state: &AppState, call: &WebCallType) -> WebReturnType {
    let path: String = service::get_arg(&call.args, 0).unwrap_or_default();
    let include_resume: bool = service::get_optional_arg(&call.args, 1).ok().flatten().unwrap_or(false);
    let store = state.mstore.clone();
    match tokio::task::spawn_blocking(move || export(&store, &path, &layout_file::ExportContext::live(include_resume))).await {
        Ok(Ok(v)) => WebReturnType::success(v),
        Ok(Err(e)) => WebReturnType::error(e),
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

/// `workspace.PreviewImport(path)`.
pub(crate) async fn handle_preview(state: &AppState, call: &WebCallType) -> WebReturnType {
    let path: String = service::get_arg(&call.args, 0).unwrap_or_default();
    let store = state.mstore.clone();
    match tokio::task::spawn_blocking(move || preview(&store, &path, &ImportTrust::live())).await {
        Ok(Ok(v)) => WebReturnType::success(v),
        Ok(Err(e)) => WebReturnType::error(e),
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

/// `workspace.ImportUserWorkspace(path, runCommands, windowId)`: the reply of
/// `SwitchUserWorkspace` plus `notes`.
pub(crate) async fn handle_import(state: &AppState, call: &WebCallType) -> WebReturnType {
    let path: String = service::get_arg(&call.args, 0).unwrap_or_default();
    let run: Option<bool> = service::get_optional_arg(&call.args, 1).ok().flatten();
    let window_id: String = service::get_arg(&call.args, 2).unwrap_or_default();
    match import(state, &path, run, &window_id, &ImportTrust::live()).await {
        Ok((switched, notes)) => {
            let mut data = switched.to_json();
            data["notes"] = json!(notes);
            WebReturnType::success_data_updates(data, super::user_workspace_switch::client_update(&state.mstore))
        }
        Err(e) => WebReturnType::error(e),
    }
}

#[cfg(test)]
#[path = "user_workspace_files_tests.rs"]
mod tests;
