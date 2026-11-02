// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Renaming an agent's displayed name: the one path the window's `renameagentdefinitiontitle` and
//! an agent's `Agents rename` both take, so every place that shows or addresses the agent follows.
//!
//! Deliberately separate from `updateagent`, which preserves `branch_label` unconditionally
//! (`core.rs`: "parent_id + branch_label describe provenance and are immutable post-insert"). That
//! stays true for every other caller of `updateagent`; this is the one narrow, explicit path that
//! may change it, and only the field the tab strip's `titleOf()` displays: `branch_label` when the
//! row already has one (a fork), else `name` (a lineage root).
//! See SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4.

use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;
use agentmux_common::slug::agent_route_name;

use super::super::AppState;
use super::template::{former_name_of_another, renamed_note};

/// Why a rename did not happen.
#[derive(Debug)]
pub(crate) enum RenameError {
    /// No such agent.
    NotFound(String),
    /// The name is not one an agent can have.
    Invalid(String),
    /// Another agent has, or answers to, the name.
    Taken(String),
    /// The store failed.
    Failed(String),
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::NotFound(m) | Self::Invalid(m) | Self::Taken(m) | Self::Failed(m)) = self;
        f.write_str(m)
    }
}

impl From<crate::backend::storage::error::StoreError> for RenameError {
    fn from(e: crate::backend::storage::error::StoreError) -> Self {
        Self::Failed(e.to_string())
    }
}

/// Give agent `id` the displayed name `title`. Its slug, id, signing keys and workspace folder stay
/// as they are; the old name still reaches it as a former name (`display_rename`), and no other agent
/// may take a name it still answers to. `refuse_taken` also refuses a name another agent answers to now (its
/// name, slug or display name, or a slug it was renamed from): an agent renaming an agent must not make two
/// agents answer to one name.
pub(crate) async fn rename_title(state: &AppState, id: &str, title: &str, refuse_taken: bool) -> Result<AgentDefinition, RenameError> {
    let (mstore, broker, event_bus, local_url) = (&state.mstore, &state.broker, &state.event_bus, &state.local_web_url);
    let title = title.trim();
    if title.is_empty() {
        return Err(RenameError::Invalid("title must not be empty".to_string()));
    }

    let all = mstore.agent_def_list()?;
    let old = all.iter().find(|a| a.id == id).ok_or_else(|| RenameError::NotFound(format!("agent {id} not found")))?;

    if former_name_of_another(mstore, title, id) {
        return Err(RenameError::Taken(format!("{title:?} still addresses another agent")));
    }
    if refuse_taken {
        refuse_if_taken(mstore, &all, id, title)?;
    }
    let old_title = shown_name(old).to_string();
    let mut updated = old.clone();
    if !updated.branch_label.is_empty() {
        updated.branch_label = title.to_string();
    } else {
        updated.name = title.to_string();
    }

    let found = mstore.agent_def_update(&mut updated)?;
    if !found {
        return Err(RenameError::NotFound(format!("agent {id} not found")));
    }

    // "My Agents" rows render `instance_name`, not these fields, so without this the picker redisplays the
    // old name (#3262). Best-effort: the rename already landed, and a failed mirror is not a failed rename.
    if let Err(e) = mstore.instance_rename(id, title) {
        tracing::warn!(
            agent_id = %id, error = %e,
            "rename: display name not updated; the picker row may keep showing the old name"
        );
    }

    // The running pane answers to the new name now; the old one still reaches it as a former name.
    crate::backend::reactive::handler::renamed(id, &old_title, title, local_url);
    broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
    // Its reports' panes name their lead in `agent:seat` (#5382).
    super::seat::refresh_pane_seats(state).await;
    // The agent's open pane shows the new name (an earlier pane title outranks the name, so it is
    // cleared), whichever UI the rename came from, and the running agent, which otherwise keeps its old
    // name until it is launched again, is told.
    if let Some(block_id) = crate::backend::reactive::get_global_handler().block_for_uid(id) {
        show_new_name(mstore, event_bus, &block_id, title);
        if old_title.as_str() != title {
            let note = renamed_note(&old_title, title);
            crate::backend::blockcontroller::system_note_into_turn_or_hold(&block_id, &format!("renamed:{title}"), &note);
        }
    }
    Ok(updated)
}

/// Refuse `title` when another agent answers to it, as typed or as a message addressed to it is routed
/// (`Zed Bot` and `Zed-Bot` are one route), or when it is a name another agent was renamed from.
fn refuse_if_taken(mstore: &Store, all: &[AgentDefinition], id: &str, title: &str) -> Result<(), RenameError> {
    let route = agent_route_name(title);
    for spelling in [title, route.as_str()] {
        let answering = mstore.agents_matching_name(spelling)?;
        if let Some(other) = answering.iter().find(|m| m.id != id) {
            return Err(RenameError::Taken(format!("{title:?} is already the name of another agent ({}); pick a different name", other.id)));
        }
        if answering.is_empty() && mstore.former_name_reserved(spelling) {
            return Err(RenameError::Taken(format!("{title:?} is a name another agent was renamed from and stays reserved for it")));
        }
    }
    let routes_like_title = |name: &str| agent_route_name(name).eq_ignore_ascii_case(&route);
    let other = all
        .iter()
        .filter(|a| a.id != id && a.is_seeded == 0 && a.user_hidden == 0)
        .find(|a| routes_like_title(&a.name) || routes_like_title(&a.branch_label) || routes_like_title(&a.slug));
    match other {
        Some(a) => Err(RenameError::Taken(format!("{title:?} is already the name of another agent ({}); pick a different name", a.id))),
        None => Ok(()),
    }
}

/// What the tab strip shows for the agent: its fork label, else its name.
fn shown_name(a: &AgentDefinition) -> &str {
    if a.branch_label.is_empty() {
        &a.name
    } else {
        &a.branch_label
    }
}

/// Set the pane's `agentName` to `title` and clear an earlier `frame:title`.
fn show_new_name(mstore: &Store, event_bus: &crate::backend::eventbus::EventBus, block_id: &str, title: &str) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("agentName".to_string(), serde_json::json!(title));
    meta.insert("frame:title".to_string(), serde_json::Value::Null);
    match crate::server::service::update_object_meta(mstore, &format!("block:{block_id}"), &meta) {
        Ok(_) => crate::backend::blockcontroller::core::broadcast_block_update(mstore, event_bus, block_id),
        Err(e) => tracing::warn!(block_id, error = %e, "rename: the pane keeps its old title until relaunch"),
    }
}
