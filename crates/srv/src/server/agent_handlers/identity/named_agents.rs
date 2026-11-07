// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The named-agent RPCs (`listnamedagents`, `hidenamedagent`), moved out of `identity::register` unchanged as
//! named functions. They power the picker's My Agents and the Files pane, joining instance rows with the
//! definition, account and bundle names so the frontend renders without follow-ups.

use std::collections::HashMap;

use super::super::account_label::{account_line, names_by_id, Degraded};
use crate::backend::rpc_types::{CommandListNamedAgentsData, HideNamedAgentResult, NamedAgentRow};
use crate::backend::storage::store::{AgentIdentityLink, AgentInstance, Bundle, IdentityAccount, Store};
use crate::backend::storage::AgentDefinition;

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;

/// Everything a row's names are looked up in, fetched once per response: with at most 200 rows and account
/// and bundle counts in the low dozens, a linear lookup on cached lists beats per-row round-trips.
struct Lookups<'a> {
    defs: &'a [AgentDefinition],
    memories: &'a [Bundle],
    links_by_agent: HashMap<&'a str, Vec<&'a AgentIdentityLink>>,
    account_names: HashMap<&'a str, &'a str>,
    mirror_failed: bool,
}

impl Lookups<'_> {
    /// The identity name resolves off the direct agent↔account links, grouped by definition id
    /// (SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md §4 item 2).
    fn identity_name(&self, definition_id: &str) -> String {
        account_line(
            self.links_by_agent.get(definition_id).map(|v| v.as_slice()),
            &self.account_names,
            Degraded { links: false, accounts: self.mirror_failed },
        )
        .text
    }

    fn memory_name(&self, memory_id: &str) -> String {
        if memory_id.is_empty() {
            "(vanilla CLI)".to_string()
        } else {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "(missing memory)".to_string())
        }
    }

    fn definition(&self, definition_id: &str) -> Option<&AgentDefinition> {
        self.defs.iter().find(|d| d.id == definition_id)
    }
}

/// `data` is a `Value`: a malformed request means the defaults.
pub(super) fn list_named_agents(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    data: serde_json::Value,
) -> Result<Vec<NamedAgentRow>, String> {
    let cmd: CommandListNamedAgentsData = serde_json::from_value(data).unwrap_or_default();
    let limit = if cmd.limit == 0 { DEFAULT_LIMIT } else { cmd.limit.min(MAX_LIMIT) };
    let defs = mstore.agent_def_list().map_err(|e| format!("listnamedagents: agent_def_list: {e}"))?;
    let memories = id_store.bundle_list().map_err(|e| format!("listnamedagents: bundle_list: {e}"))?;
    let links = identity_store.agent_identity_list_all().map_err(|e| format!("listnamedagents: agent_identity_links: {e}"))?;
    let accounts = id_store.identity_list(None).map_err(|e| format!("listnamedagents: accounts: {e}"))?;
    let (mirror_accounts, mirror_failed) = mirror_accounts_for(&links, &accounts, identity_store);
    let mut links_by_agent: HashMap<&str, Vec<&AgentIdentityLink>> = HashMap::new();
    for link in &links {
        links_by_agent.entry(link.agent_id.as_str()).or_default().push(link);
    }
    let lookups = Lookups {
        defs: &defs,
        memories: &memories,
        links_by_agent,
        account_names: names_by_id(&accounts, &mirror_accounts),
        mirror_failed,
    };
    // Read from the cross-version registry when it's available; SQLite is the fallback when it couldn't be
    // resolved at startup (CI, odd environments). SQLite stays authoritative while the parallel write is on.
    match mstore.shared_agent_registry() {
        Some(reg) => registry_rows(mstore, &reg, &cmd, limit, &lookups),
        None => instance_rows(mstore, &cmd, limit, &lookups),
    }
}

/// The global account mirror, read only when a link names an account this channel's store doesn't hold. The
/// flag is set when that read fails, so a link that resolves to nothing is left empty, not "No auth".
fn mirror_accounts_for(links: &[AgentIdentityLink], own: &[IdentityAccount], identity_store: &Store) -> (Vec<IdentityAccount>, bool) {
    let own_ids: std::collections::HashSet<&str> = own.iter().map(|a| a.id.as_str()).collect();
    if !links.iter().any(|l| !own_ids.contains(l.account_id.as_str())) {
        return (Vec::new(), false);
    }
    match identity_store.identity_list(None) {
        Ok(accounts) => (accounts, false),
        Err(e) => {
            tracing::warn!(error = %e, "listnamedagents: the global account mirror could not be read");
            (Vec::new(), true)
        }
    }
}

fn registry_rows(
    mstore: &Store,
    reg: &crate::registry::Registry,
    cmd: &CommandListNamedAgentsData,
    limit: usize,
    lookups: &Lookups,
) -> Result<Vec<NamedAgentRow>, String> {
    // Re-join a relative working_dir against the CURRENT channel's agents dir (symmetric with the write
    // mirror), not the registry's own parent: P0.3 re-roots the registry out of channels/<ch>/agents/.
    let agents_root = mstore.registry_agents_base();
    let mut records = reg.list_active().map_err(|e| format!("listnamedagents: registry: {e}"))?;
    if let Some(def_filter) = cmd.definition_id.as_deref() {
        records.retain(|r| r.data.definition_id == def_filter);
    }
    records.sort_by(|a, b| b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms));
    records.truncate(limit);
    // Pre-fetch the candidate same-version rows ONCE so enrichment doesn't issue N+1 queries, indexed by
    // instance_id; rows that aren't in current SQLite fall through to sentinels. Head-of-chain only: the
    // registry mirror excludes continuations (see `registry_upsert_if_named`), so the SQLite side must match,
    // else continuation rows displace registry-head rows under the `limit` and the merge-by-id enrichment
    // misses, silently downgrading running-state badges and block_id_hints to "available" / empty.
    let sqlite_rows: Vec<AgentInstance> = mstore
        .instance_list_named(records.len().max(1), cmd.definition_id.as_deref(), /* identity_id */ None, /* include_continuations */ false)
        .unwrap_or_default();
    let sqlite_by_id: HashMap<&str, &AgentInstance> = sqlite_rows.iter().map(|i| (i.id.as_str(), i)).collect();
    Ok(records
        .into_iter()
        .map(|rec| {
            let d = rec.data;
            let def = lookups.definition(&d.definition_id);
            let memory_id = d.memory_id.clone().unwrap_or_default();
            // v3 records carry their SOURCE channel agents dir, so a row from another channel resolves to its
            // real workspace; legacy (v1/v2) records fall back to the current channel base (the pre-P0.4
            // behaviour, correct for the same-channel rows v1/v2 could represent).
            let working_directory = if let Some(src) = d.source_agents_base.as_deref() {
                std::path::Path::new(src).join(&d.working_dir).to_string_lossy().to_string()
            } else {
                match agents_root.as_ref() {
                    Some(root) => root.join(&d.working_dir).to_string_lossy().to_string(),
                    None => d.working_dir.clone(),
                }
            };
            // Same-version enrichment: a row that is also in current SQLite carries runtime state (block_id for
            // focus-existing-pane, status, ended_at) the registry intentionally doesn't track. Cross-version
            // rows fall through with the "available" status and an empty block_id_hint.
            let (block_id_hint, status, ended_at) = match sqlite_by_id.get(d.instance_id.as_str()) {
                Some(inst) => (inst.block_id.clone(), inst.status.clone(), inst.ended_at),
                None => (String::new(), "available".to_string(), 0),
            };
            NamedAgentRow {
                instance_id: d.instance_id,
                instance_name: d.instance_name,
                definition_id: d.definition_id.clone(),
                definition_name: def.map(|x| x.name.clone()).unwrap_or_else(|| "(missing definition)".to_string()),
                provider: def.map(|x| x.provider.clone()).unwrap_or_default(),
                working_directory,
                identity_id: d.identity_id.clone().unwrap_or_default(),
                identity_name: lookups.identity_name(&d.definition_id),
                memory_name: lookups.memory_name(&memory_id),
                memory_id,
                started_at: d.last_launched_at_ms,
                ended_at,
                status,
                block_id_hint,
            }
        })
        .collect())
}

/// No-registry fallback: drives the launch modal's "Continue agent" dropdown directly. One entry per chain
/// root, mirroring the registry path's semantics.
fn instance_rows(mstore: &Store, cmd: &CommandListNamedAgentsData, limit: usize, lookups: &Lookups) -> Result<Vec<NamedAgentRow>, String> {
    let instances = mstore
        .instance_list_named(limit, cmd.definition_id.as_deref(), /* identity_id */ None, /* include_continuations */ false)
        .map_err(|e| format!("listnamedagents: {e}"))?;
    Ok(instances
        .into_iter()
        .map(|inst| {
            let def = lookups.definition(&inst.definition_id);
            NamedAgentRow {
                definition_name: def.map(|d| d.name.clone()).unwrap_or_else(|| "(missing definition)".to_string()),
                provider: def.map(|d| d.provider.clone()).unwrap_or_default(),
                identity_name: lookups.identity_name(&inst.definition_id),
                memory_name: lookups.memory_name(&inst.memory_id),
                instance_id: inst.id,
                instance_name: inst.instance_name,
                definition_id: inst.definition_id,
                working_directory: inst.working_directory,
                identity_id: inst.identity_id,
                memory_id: inst.memory_id,
                started_at: inst.started_at,
                ended_at: inst.ended_at,
                status: inst.status,
                block_id_hint: inst.block_id,
            }
        })
        .collect())
}

/// Soft-delete (`display_hidden = 1`) so the row disappears from the dropdown. The working dir stays on disk.
pub(super) fn hide_named_agent(
    mstore: &Store,
    broker: &crate::backend::mps::Broker,
    id: &str,
) -> Result<HideNamedAgentResult, String> {
    let hidden = mstore.instance_set_hidden(id, true).map_err(|e| format!("hidenamedagent: {e}"))?;
    if hidden {
        broker.publish_event::<crate::backend::mps_events::NamedAgentsChanged>(&(), vec![], 0);
    }
    Ok(HideNamedAgentResult { hidden })
}
