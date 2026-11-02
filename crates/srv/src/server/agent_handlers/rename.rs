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

use super::super::AppState;
use super::template::{former_name_of_another, renamed_note};

/// Give agent `id` the displayed name `title`. Its slug, id, signing keys and workspace folder stay
/// as they are; the old name still reaches it as a former name (`display_rename`), and no other agent
/// may take a name it still answers to. `refuse_taken` also refuses a name another agent has now:
/// an agent renaming an agent must not make two agents answer to one name.
pub(crate) async fn rename_title(state: &AppState, id: &str, title: &str, refuse_taken: bool) -> Result<AgentDefinition, String> {
    let (mstore, broker, event_bus, local_url) = (&state.mstore, &state.broker, &state.event_bus, &state.local_web_url);
    let title = title.trim();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }

    let all = mstore.agent_def_list().map_err(|e| e.to_string())?;
    let old = all.iter().find(|a| a.id == id).ok_or_else(|| format!("agent {id} not found"))?;

    if former_name_of_another(mstore, title, id) {
        return Err(format!("{title:?} still addresses another agent"));
    }
    if refuse_taken {
        if let Some(other) = all.iter().find(|a| a.id != id && shown_name(a).eq_ignore_ascii_case(title)) {
            return Err(format!("{title:?} is already the name of another agent ({}); pick a different name", other.id));
        }
    }
    let old_title = shown_name(old).to_string();
    let mut updated = old.clone();
    if !updated.branch_label.is_empty() {
        updated.branch_label = title.to_string();
    } else {
        updated.name = title.to_string();
    }

    let found = mstore.agent_def_update(&mut updated).map_err(|e| e.to_string())?;
    if !found {
        return Err(format!("agent {id} not found"));
    }

    // The fields above are what the pane tab strip's `titleOf()` reads. "My Agents" rows render
    // `instance_name` instead, so without this a rename from the picker closed its editor, refetched,
    // and redisplayed the OLD name (codex P1 on PR #3262). See `Store::instance_rename` for why it's
    // scoped to rows that already carry an `instance_name`.
    //
    // This also applies to the tab-strip rename, deliberately: since the agent-concept consolidation a
    // fork tab and its picker row are ONE agent row, so letting the two names drift apart is the
    // confusing outcome, not the safe one. Best-effort: the title change already landed, and a
    // display-name mirror failing is not a failed rename.
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
