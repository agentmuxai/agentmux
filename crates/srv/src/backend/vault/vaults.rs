// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Several Vaults on one computer: the person keeps some entries to one set of agents
//! and others to another, or to none
//! (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5).
//!
//! Every entry belongs to exactly one Vault. Entry ids stay global, so every route that
//! names an entry by id works as before; a Vault only narrows which agents may ask for
//! an entry, and an agent that may not is told the entry doesn't exist.

use serde::{Deserialize, Serialize};

use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultInfoRow, PERSONAL_VAULT};

use super::{record, PERSON};

const MAX_NAME: usize = 60;
const MAX_VAULTS: usize = 50;

/// Which agents may use a Vault's entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "mode", content = "agents", rename_all = "lowercase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum VaultAgents {
    /// Every agent on this machine (Personal's setting, today's behaviour).
    All,
    /// None: the person uses the entries themselves (reveal, copy, a browser fill they approve).
    None,
    /// Only these agents, by id.
    Named(Vec<String>),
}

/// A Vault as the pane shows it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    pub agents: VaultAgents,
    pub entries: u32,
}

fn agents_of(row: &VaultInfoRow) -> VaultAgents {
    match row.agents.as_str() {
        "none" => VaultAgents::None,
        "named" => VaultAgents::Named(serde_json::from_str(&row.agent_ids).unwrap_or_default()),
        _ => VaultAgents::All,
    }
}

fn columns(agents: &VaultAgents) -> (&'static str, String) {
    match agents {
        VaultAgents::All => ("all", "[]".into()),
        VaultAgents::None => ("none", "[]".into()),
        VaultAgents::Named(ids) => ("named", serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Whether `agent` may use the entries of the Vault `row`. An agent that isn't proven
/// (`None`) is let into a Vault open to all agents only.
pub fn may_use(row: &VaultInfoRow, agent: Option<&str>) -> bool {
    match agents_of(row) {
        VaultAgents::All => true,
        VaultAgents::None => false,
        VaultAgents::Named(ids) => agent.is_some_and(|a| ids.iter().any(|id| same(id, a))),
    }
}

/// Whether `agent` may use the entry `entry_id`. An entry in a Vault that is gone counts
/// as Personal's.
pub fn entry_usable(store: &Store, entry_id: &str, agent: Option<&str>) -> Result<bool, String> {
    let Some(row) = store.vault_get(entry_id).map_err(|e| e.to_string())? else { return Ok(false) };
    entry_row_usable(store, &row.vault_id, agent)
}

/// [`entry_usable`] for a Vault id.
pub fn entry_row_usable(store: &Store, vault_id: &str, agent: Option<&str>) -> Result<bool, String> {
    Ok(store.vault_info(vault_id).map_err(|e| e.to_string())?.is_none_or(|v| may_use(&v, agent)))
}

/// The Vault a new entry made by `agent` goes to when it names none: the first Vault, in
/// the person's order, the agent may use.
pub fn default_for(store: &Store, agent: Option<&str>) -> Result<String, String> {
    store
        .vaults()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|v| may_use(v, agent))
        .map(|v| v.id)
        .ok_or_else(|| "no Vault is open to this agent: the person decides which agents each Vault is for, in the Vault pane".to_string())
}

/// A Vault by name or id. `agent` limits it to the Vaults that agent may use; an agent
/// told "no such Vault" can't tell a Vault it may not use from one that isn't there.
pub fn resolve(store: &Store, which: &str, agent: Option<Option<&str>>) -> Result<VaultInfoRow, String> {
    let which = which.trim();
    let vaults = store.vaults().map_err(|e| e.to_string())?;
    let found = vaults
        .iter()
        .find(|v| v.id == which)
        .or_else(|| vaults.iter().find(|v| same(&v.name, which)))
        .filter(|v| agent.is_none_or(|a| may_use(v, a)))
        .cloned();
    found.ok_or_else(|| {
        let names: Vec<&str> = vaults.iter().filter(|v| agent.is_none_or(|a| may_use(v, a))).map(|v| v.name.as_str()).collect();
        format!("no Vault {which:?}; the Vaults are {}", if names.is_empty() { "none".to_string() } else { names.join(", ") })
    })
}

/// Every Vault, with how many entries it holds.
pub fn list(store: &Store) -> Result<Vec<VaultInfo>, String> {
    let entries = store.vault_list().map_err(|e| e.to_string())?;
    Ok(store
        .vaults()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|v| VaultInfo { entries: entries.iter().filter(|e| e.vault_id == v.id).count() as u32, agents: agents_of(&v), id: v.id, name: v.name })
        .collect())
}

fn clean_name(store: &Store, name: &str, besides: Option<&str>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a Vault needs a name".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a Vault's name is at most {MAX_NAME} characters"));
    }
    if store.vaults().map_err(|e| e.to_string())?.iter().any(|v| Some(v.id.as_str()) != besides && same(&v.name, name)) {
        return Err(format!("there is already a Vault named {name:?}"));
    }
    Ok(name.to_string())
}

fn clean_agents(agents: VaultAgents) -> Result<VaultAgents, String> {
    let VaultAgents::Named(ids) = agents else { return Ok(agents) };
    let mut out: Vec<String> = Vec::new();
    for id in ids.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
        if id.chars().count() > 64 {
            return Err("an agent id is at most 64 characters".into());
        }
        if !out.iter().any(|o| same(o, id)) {
            out.push(id.to_string());
        }
    }
    Ok(VaultAgents::Named(out))
}

/// Adds a Vault with a name and who may use it.
pub fn create(store: &Store, name: &str, agents: VaultAgents) -> Result<VaultInfo, String> {
    if store.vaults().map_err(|e| e.to_string())?.len() >= MAX_VAULTS {
        return Err(format!("at most {MAX_VAULTS} Vaults"));
    }
    let name = clean_name(store, name, None)?;
    let agents = clean_agents(agents)?;
    let (mode, ids) = columns(&agents);
    let id = format!("vault-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let row = VaultInfoRow { id: id.clone(), name: name.clone(), agents: mode.into(), agent_ids: ids, position: 0, created_at: agentmux_common::time::now_ms() };
    store.vault_add(&row).map_err(|e| e.to_string())?;
    record(store, "", &name, PERSON, "created a Vault", &name, "done");
    Ok(VaultInfo { id, name, agents, entries: 0 })
}

/// Renames a Vault and sets who may use it.
pub fn update(store: &Store, id: &str, name: &str, agents: VaultAgents) -> Result<(), String> {
    store.vault_info(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no Vault {id}"))?;
    let name = clean_name(store, name, Some(id))?;
    let agents = clean_agents(agents)?;
    let (mode, ids) = columns(&agents);
    store.vault_set(id, &name, mode, &ids).map_err(|e| e.to_string())?;
    record(store, "", &name, PERSON, "changed a Vault", &name, "done");
    Ok(())
}

/// Removes an empty Vault. Personal stays.
pub fn remove(store: &Store, id: &str) -> Result<(), String> {
    if id == PERSONAL_VAULT {
        return Err("Personal is the Vault every entry started in, and stays".into());
    }
    let info = store.vault_info(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no Vault {id}"))?;
    if !store.vault_remove(id).map_err(|e| e.to_string())? {
        return Err(format!("{} still holds entries: move them to another Vault, or delete them, first", info.name));
    }
    record(store, "", &info.name, PERSON, "deleted a Vault", &info.name, "done");
    Ok(())
}

/// Puts an entry in another Vault, keeping its id, versions and grants. Only the person
/// does this: a move can open a value to more agents.
pub fn move_entry(store: &Store, entry_id: &str, vault_id: &str) -> Result<(), String> {
    let row = store.vault_get(entry_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {entry_id}"))?;
    let to = store.vault_info(vault_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no Vault {vault_id}"))?;
    if row.vault_id == to.id {
        return Ok(());
    }
    let from = store.vault_info(&row.vault_id).map_err(|e| e.to_string())?.map(|v| v.name).unwrap_or_else(|| row.vault_id.clone());
    store.vault_move_entry(entry_id, &to.id).map_err(|e| e.to_string())?;
    record(store, entry_id, &row.label, PERSON, "moved to another Vault", &format!("{from} -> {}", to.name), "done");
    Ok(())
}

#[cfg(test)]
mod tests;
