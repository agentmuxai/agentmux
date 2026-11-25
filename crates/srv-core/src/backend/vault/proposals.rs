// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's request to change where a login is offered, on a login it did not make
//! (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md R8).
//!
//! The call returns at once and changes nothing: the login keeps its site, realm and proxy in every way, so the person's own
//! sign-ins at the old place keep working. The request waits 7 days as a record, one per entry and field, a newer one
//! replacing the older. The person accepts it (it applies only if the entry is as it was when proposed) or dismisses it.
//! Nothing here asks anyone and nothing here holds a value.

use std::collections::BTreeMap;

use agentmux_common::time::now_ms;

use super::{history, kinds, record_line, signins, validate, Line, Parts, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault_changes::{DetailsKept, Proposal};

/// How long a proposal waits.
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// What the person is told when the proposal they were shown has been replaced by another.
pub const REPLACED: &str = "the agent changed its proposal since this was shown: look at the new one";
/// The Activity action of an agent's proposal.
pub const PROPOSED: &str = "proposed a change";
/// The Activity action of the person accepting one.
pub const ACCEPTED: &str = "accepted a proposed change";
/// The Activity action of the person dismissing one.
pub const DISMISSED: &str = "dismissed a proposed change";

/// The fields of a login that say where it is offered and that an agent may propose a change of. `profile` is not among
/// them: a login usable in an agent's browser is a share, and a share asks.
pub const FIELDS: [&str; 3] = [signins::SITE, signins::REALM, signins::PROXY];

/// Records that `agent` asks for `field` of the login `id` to become `value` (empty: removed), with its reason, and writes
/// the line that says so. The login is unchanged.
pub fn propose(store: &Store, id: &str, field: &str, value: &str, agent: &str, reason: &str) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let proposal = Proposal {
        entry_id: row.id.clone(),
        field: field.to_string(),
        entry_created_at: row.created_at,
        entry_updated_at: row.updated_at,
        value: value.trim().to_string(),
        agent: agent.to_string(),
        reason: reason.to_string(),
        at: now_ms(),
    };
    store.vault_proposal_put(&proposal).map_err(|e| e.to_string())?;
    let target = format!("field {field}");
    record_line(store, Line { entry_id: &row.id, label: &row.label, actor: agent, action: PROPOSED, target: &target, outcome: "waiting for the person", reason });
    Ok(())
}

/// The proposals still waiting, oldest first; older ones are dropped first.
pub fn list(store: &Store) -> Result<Vec<Proposal>, String> {
    if let Err(e) = store.vault_proposals_prune(now_ms() - KEEP_MS) {
        tracing::warn!(error = %e, "vault: couldn't delete old proposals");
    }
    store.vault_proposals(None, now_ms() - KEEP_MS).map_err(|e| e.to_string())
}

fn waiting(store: &Store, id: &str, field: &str) -> Result<Proposal, String> {
    let all = store.vault_proposals(Some(id), now_ms() - KEEP_MS).map_err(|e| e.to_string())?;
    all.into_iter().find(|p| p.field == field).ok_or_else(|| format!("no proposal for {field} of {id}: it was accepted, dismissed or is older than 7 days"))
}

/// The person accepts the proposal for `field` of `id`: it is applied if the entry is as it was when proposed, with the
/// details it replaces kept 7 days, and a line records it. An entry changed since is refused, saying so.
pub fn accept(store: &Store, id: &str, field: &str) -> Result<(), String> {
    accept_shown(store, id, field, None)
}

/// [`accept`] of the proposal the person was shown, made at `shown_at`: refused when an agent has replaced it since, so what
/// is applied is what they saw. `None` accepts whichever is waiting.
pub fn accept_shown(store: &Store, id: &str, field: &str, shown_at: Option<i64>) -> Result<(), String> {
    let p = waiting(store, id, field)?;
    if shown_at.is_some_and(|at| at != p.at) {
        return Err(REPLACED.into());
    }
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    if row.created_at != p.entry_created_at || row.updated_at != p.entry_updated_at {
        return Err("the login changed after this was proposed: dismiss it, and the agent can propose again".into());
    }
    let mut fields: BTreeMap<String, String> = signins::fields_of(&row);
    match p.value.as_str() {
        "" => fields.remove(field),
        v => fields.insert(field.to_string(), v.to_string()),
    };
    signins::normalize_site(&row.kind, &mut fields);
    // The same checks an edit passes: a value that was never valid for the field is not written.
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    validate(kind, &row.label, &fields, &signins::tags_of(&row), &Parts::default())?;
    let was = (row.sealed.clone(), row.fields.clone());
    let kept = DetailsKept {
        entry_id: row.id.clone(),
        entry_created_at: row.created_at,
        label: row.label.clone(),
        fields: row.fields.clone(),
        tags: row.tags.clone(),
        changed_at: now_ms(),
        changed_by: PERSON.to_string(),
        what: format!("field {field}"),
    };
    let new = crate::backend::storage::vault::VaultRow { fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?, updated_at: now_ms().max(row.updated_at + 1), ..row.clone() };
    if !store.vault_update_details_kept(&new, &was.0, &was.1, &kept, None).map_err(|e| e.to_string())? {
        return Err("the login changed while this was being applied, so nothing was changed: look at it again".into());
    }
    let _ = store.vault_proposal_delete(id, field);
    // The old place's grants name the old origin and stop matching by themselves; a new place starts with none.
    let target = format!("field {field}");
    let reason = format!("{} asked: {}", history::actor_label(&p.agent), p.reason);
    record_line(store, Line { entry_id: &row.id, label: &row.label, actor: PERSON, action: ACCEPTED, target: &target, outcome: "done", reason: &reason });
    Ok(())
}

/// The person dismisses the proposal for `field` of `id`. True when there was one.
pub fn dismiss(store: &Store, id: &str, field: &str) -> Result<bool, String> {
    dismiss_shown(store, id, field, None)
}

/// [`dismiss`] of the proposal the person was shown (see [`accept_shown`]): a replacement is left waiting to be looked at.
pub fn dismiss_shown(store: &Store, id: &str, field: &str, shown_at: Option<i64>) -> Result<bool, String> {
    let p = match waiting(store, id, field) {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    if shown_at.is_some_and(|at| at != p.at) {
        return Err(REPLACED.into());
    }
    let row = store.vault_get(id).map_err(|e| e.to_string())?;
    let gone = store.vault_proposal_delete(id, field).map_err(|e| e.to_string())?;
    if let (true, Some(row)) = (gone, row) {
        let target = format!("field {field}");
        record_line(store, Line { entry_id: &row.id, label: &row.label, actor: PERSON, action: DISMISSED, target: &target, outcome: "done", reason: &p.reason });
    }
    Ok(gone)
}
