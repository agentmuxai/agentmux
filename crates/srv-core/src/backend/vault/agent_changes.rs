// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the person sees instead of a card: the changes agents made that they have not looked at yet, and what they can do
//! about each (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md §5).
//!
//! A change is a history line an agent wrote. The list is read from the history, so it cannot disagree with it. *Seen* is a
//! mark on this machine only, and it also trusts the value or place the agent set (`agent_set`). *Undo* puts back what the
//! change replaced (the earlier value, the earlier details) or removes an entry an agent added, as the person's own action.
//! Nothing here holds a value.

use std::collections::BTreeMap;

use serde::Serialize;

use agentmux_common::time::now_ms;

use super::{agent_set, expiry, history, inspect, proposals, record_line, versions, Line, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};
use crate::backend::storage::vault_changes::DetailsKept;

/// How far back the list reaches.
const LOOK_BACK_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// The most changes it lists.
const MOST: i64 = 300;
/// How near in time an earlier value or earlier details are to the line that replaced them: they are written in one call.
const NEAR_MS: i64 = 60_000;
/// The Activity action of the person putting earlier details back.
pub const RESTORED_DETAILS: &str = "restored earlier details";

/// The lines of an agent that are changes to look at.
const ACTIONS: [&str; 5] = ["generated", "imported", "captured", inspect::CHANGED_DETAILS, proposals::PROPOSED];

/// A pending proposal's field, what is proposed and what the login has now.
#[derive(Debug, Clone, Serialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ProposalInfo {
    pub field: String,
    pub value: String,
    pub current: String,
}

/// One change by an agent.
#[derive(Debug, Clone, Serialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentChange {
    /// The history line's number on this machine: what *Seen* and *Undo* name.
    #[ts(type = "number")]
    pub seq: i64,
    #[ts(type = "number")]
    pub at: i64,
    /// The agent's id, as the history writes it after `agent:`.
    pub agent: String,
    pub entry_id: String,
    pub entry_label: String,
    /// `added`, `new value`, `changed details` or `proposal`.
    pub kind: String,
    /// The history line's action (`generated`, `imported`, `captured`, `changed details`) and its target in names.
    pub action: String,
    pub target: String,
    /// The agent's reason, as it wrote it: text, never markup.
    pub reason: String,
    pub seen: bool,
    /// What *Undo* does: `remove` (an entry the agent added), `restore value`, `restore details`, or empty.
    pub undo: String,
    /// A new value on an entry the person made.
    pub replaces_yours: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub proposal: Option<ProposalInfo>,
}

fn agent_of(actor: &str) -> String {
    actor.strip_prefix("agent:").unwrap_or(actor).to_string()
}

fn is_agent(row: &VaultActivityRow, agent_label: &str) -> bool {
    row.actor_kind == "agent" && history::actor_label(&row.actor) == agent_label
}

/// The earlier value `row` replaced, while it is still kept.
fn earlier_value(store: &Store, entry: &VaultRow, row: &VaultActivityRow) -> Option<i64> {
    let label = history::actor_label(&row.actor);
    store
        .vault_versions(&entry.id, entry.created_at)
        .ok()?
        .into_iter()
        .find(|v| v.state == versions::PREVIOUS && history::actor_label(&v.replaced_by) == label && v.replaced_at <= row.at + 1000 && row.at - v.replaced_at <= NEAR_MS)
        .map(|v| v.seq)
}

/// The details `row` replaced, while they are still kept.
fn earlier_details(store: &Store, entry: &VaultRow, row: &VaultActivityRow) -> Option<DetailsKept> {
    let label = history::actor_label(&row.actor);
    store.vault_details_kept(&entry.id, entry.created_at).ok()?.into_iter().find(|d| d.changed_by == label && d.what == row.target && d.changed_at <= row.at + 1000 && row.at - d.changed_at <= NEAR_MS)
}

/// `row` as a change the person can look at, or none when it is not one now (the entry is gone, the proposal is settled).
fn change_of(store: &Store, row: &VaultActivityRow, seen: bool) -> Option<AgentChange> {
    let entry = store.vault_get(&row.entry_id).ok()??;
    let first = store.vault_activity_first(&entry.id, entry.created_at).ok()?;
    let by_agent = first.as_ref().is_some_and(|f| f.actor_kind == "agent");
    let added = first.as_ref().is_some_and(|f| f.seq == row.seq);
    let mut change = AgentChange {
        seq: row.seq,
        at: row.at,
        agent: agent_of(&row.actor),
        entry_id: entry.id.clone(),
        entry_label: entry.label.clone(),
        kind: String::new(),
        action: row.action.clone(),
        target: row.target.clone(),
        reason: row.reason.clone(),
        seen,
        undo: String::new(),
        replaces_yours: false,
        proposal: None,
    };
    let recent = now_ms() - row.at <= versions::KEEP_MS;
    if row.action == proposals::PROPOSED {
        let field = row.target.strip_prefix("field ").unwrap_or(&row.target).to_string();
        let waiting = store.vault_proposals(Some(&entry.id), now_ms() - proposals::KEEP_MS).ok()?.into_iter().find(|p| p.field == field && p.at <= row.at + 1000 && p.at >= row.at - NEAR_MS)?;
        let current = signins_field(&entry, &field);
        change.kind = "proposal".into();
        change.proposal = Some(ProposalInfo { field, value: waiting.value, current });
    } else if added {
        change.kind = "added".into();
        change.undo = "remove".into();
    } else if row.action == inspect::CHANGED_DETAILS {
        change.kind = "changed details".into();
        if recent && earlier_details(store, &entry, row).is_some() {
            change.undo = "restore details".into();
        }
    } else {
        change.kind = "new value".into();
        change.replaces_yours = !by_agent;
        if recent && earlier_value(store, &entry, row).is_some() {
            change.undo = "restore value".into();
        }
    }
    Some(change)
}

fn signins_field(entry: &VaultRow, field: &str) -> String {
    super::signins::fields_of(entry).get(field).cloned().unwrap_or_default()
}

/// The changes by agents, newest first, of one entry (`entry`) or all, with the ones the person has marked seen left out
/// unless `include_seen`. A proposal, and a new value on an entry the person made, come first.
pub fn list(store: &Store, entry: Option<&str>, include_seen: bool) -> Result<Vec<AgentChange>, String> {
    versions::prune(store);
    let rows = store.vault_activity_agent_changes(now_ms() - LOOK_BACK_MS, &ACTIONS, MOST).map_err(|e| e.to_string())?;
    let (through, marked) = store.vault_changes_seen().map_err(|e| e.to_string())?;
    let mut out: Vec<AgentChange> = rows
        .iter()
        .filter(|r| entry.is_none_or(|id| r.entry_id == id))
        .filter_map(|r| change_of(store, r, r.seq <= through || marked.contains(&r.seq)))
        .filter(|c| include_seen || !c.seen)
        .collect();
    out.sort_by_key(|c| (!(c.kind == "proposal" || c.replaces_yours), std::cmp::Reverse(c.seq)));
    Ok(out)
}

/// A change that can have left the entry's unseen mark: a new entry, a new value, or a changed place. A new label, tags or a
/// plain field do not, and a proposal sets nothing until it is accepted.
fn sets_the_mark(c: &AgentChange) -> bool {
    match c.kind.as_str() {
        "added" | "new value" => true,
        "changed details" => c.target.split(", ").any(|t| matches!(t, "field site" | "field realm" | "field proxy")),
        _ => false,
    }
}

/// The value or place an agent set is trusted once the person has dealt with every change by an agent on the entry: the mark
/// is the entry's, not one change's, so a newer change that is still listed keeps it.
fn trust_when_none_left(store: &Store, entry_id: &str) -> Result<(), String> {
    if !list(store, Some(entry_id), false)?.iter().any(sets_the_mark) {
        agent_set::clear(store, entry_id);
    }
    Ok(())
}

/// The person has looked at the change recorded by line `seq` (one an agent wrote): it leaves the list, and the value or place
/// the agent set is trusted from now on, as if they had answered a use of it, once no other change by an agent on the entry
/// is left unseen.
pub fn seen(store: &Store, seq: i64) -> Result<(), String> {
    let row = store.vault_activity_get(seq).map_err(|e| e.to_string())?.filter(|r| r.actor_kind == "agent").ok_or("no such change by an agent")?;
    store.vault_change_seen(seq).map_err(|e| e.to_string())?;
    trust_when_none_left(store, &row.entry_id)
}

/// [`seen`] for every change listed now except a proposal, which waits for Accept or Dismiss and so stays listed.
pub fn seen_all(store: &Store) -> Result<(), String> {
    let all = list(store, None, false)?;
    let floor = all.iter().filter(|c| c.kind == "proposal").map(|c| c.seq).min();
    let mut through = 0;
    for c in all.iter().filter(|c| c.kind != "proposal") {
        agent_set::clear(store, &c.entry_id);
        if floor.is_none_or(|f| c.seq < f) {
            through = through.max(c.seq);
        } else {
            store.vault_change_seen(c.seq).map_err(|e| e.to_string())?;
        }
    }
    if through > 0 {
        store.vault_changes_seen_all(through).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The person puts back what the change recorded by line `seq` replaced, or removes the entry it added: their own action,
/// recorded as theirs. Refused when there is nothing to put back (past its 7 days).
pub fn undo(store: &Store, seq: i64) -> Result<(), String> {
    let row = store.vault_activity_get(seq).map_err(|e| e.to_string())?.filter(|r| r.actor_kind == "agent").ok_or("no such change by an agent")?;
    let change = change_of(store, &row, false).ok_or("that change is not there any more")?;
    let entry = store.vault_get(&change.entry_id).map_err(|e| e.to_string())?.ok_or("that entry is gone")?;
    match change.undo.as_str() {
        "remove" => super::delete(store, &entry.id)?,
        "restore value" => {
            let version = earlier_value(store, &entry, &row).ok_or("the earlier value is past its 7 days")?;
            versions::restore(store, &entry.id, version)?;
        }
        "restore details" => restore_details(store, &entry, &row)?,
        _ => return Err("there is nothing to put back for that change: the earlier version is past its 7 days".into()),
    }
    store.vault_change_seen(seq).map_err(|e| e.to_string())?;
    trust_when_none_left(store, &change.entry_id)
}

/// Puts back only what the agent's change touched (the names in its line's target), so a later edit of something else by the
/// person is kept, and drops the kept row it read so the same change cannot be undone twice.
fn restore_details(store: &Store, entry: &VaultRow, row: &VaultActivityRow) -> Result<(), String> {
    let kept = earlier_details(store, entry, row).ok_or("the earlier details are past their 7 days")?;
    let earlier: BTreeMap<String, String> = serde_json::from_str(&kept.fields).unwrap_or_default();
    let mut fields = super::signins::fields_of(entry);
    let (mut label, mut tags) = (entry.label.clone(), entry.tags.clone());
    let mut put_back = |name: &str| {
        match earlier.get(name) {
            Some(v) => fields.insert(name.to_string(), v.clone()),
            None => fields.remove(name),
        };
    };
    for part in row.target.split(", ") {
        match part {
            "label" => label = kept.label.clone(),
            "tags" => tags = kept.tags.clone(),
            "no expiry date" => put_back(expiry::FIELD),
            p if p.starts_with("expires ") => put_back(expiry::FIELD),
            p => {
                if let Some(name) = p.strip_prefix("field ") {
                    put_back(name);
                }
            }
        }
    }
    let now = DetailsKept {
        entry_id: entry.id.clone(),
        entry_created_at: entry.created_at,
        label: entry.label.clone(),
        fields: entry.fields.clone(),
        tags: entry.tags.clone(),
        changed_at: now_ms(),
        changed_by: PERSON.to_string(),
        what: RESTORED_DETAILS.to_string(),
    };
    let fields_json = serde_json::to_string(&fields).map_err(|e| e.to_string())?;
    let changed = VaultRow { label: label.clone(), fields: fields_json, tags, updated_at: now_ms().max(entry.updated_at + 1), ..entry.clone() };
    if !store.vault_update_details_kept(&changed, &entry.sealed, &entry.fields, &now, None).map_err(|e| e.to_string())? {
        return Err(format!("{} changed while this was being put back: nothing was changed", entry.label));
    }
    store.vault_details_kept_delete(&kept).map_err(|e| e.to_string())?;
    record_line(store, Line { entry_id: &entry.id, label: &label, actor: PERSON, action: RESTORED_DETAILS, target: &row.target, outcome: "done", reason: "" });
    Ok(())
}
