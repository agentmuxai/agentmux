// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agents Warden's History section lists (`history.agents`): every agent
//! whose history the global transcript store holds, with how much of it is
//! current and how much archived. Read from the store's file records only,
//! never the transcripts, so it stays fast however large they are. An agent
//! that was deleted keeps its history and is listed too.

use std::collections::BTreeMap;

use crate::backend::agent_session::OUTPUT_FILE;
use crate::backend::rpc_types::HistoryAgent;
use crate::backend::storage::filestore::FileStore;

/// What the agent registry says about a definition id.
#[derive(Debug, Clone, Default)]
pub struct AgentInfo {
    pub name: String,
    pub provider: String,
    /// The definition it was forked from; empty for an agent that isn't a fork.
    pub fork_of: String,
}

/// Which agent a zone belongs to, and whether it's an archive.
fn agent_zone(zone: &str) -> Option<(&str, bool)> {
    let rest = zone.strip_prefix("agent:")?;
    if let Some(id) = rest.strip_suffix(":current") {
        return (!id.is_empty() && !id.contains(':')).then_some((id, false));
    }
    let (id, ms) = rest.split_once(":archive:")?;
    let valid = !id.is_empty() && !id.contains(':') && !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit());
    valid.then_some((id, true))
}

/// Every agent with history in `store`, most recently active first. `info`
/// looks a definition id up in the agent registry; `None` is a deleted agent.
pub fn agents(store: &FileStore, info: impl Fn(&str) -> Option<AgentInfo>) -> Result<Vec<HistoryAgent>, String> {
    let zones = store.zone_ids_with_prefix("agent:").map_err(|e| format!("listing the transcript store: {e}"))?;
    let mut by_id: BTreeMap<String, HistoryAgent> = BTreeMap::new();
    for zone in &zones {
        let Some((id, archived)) = agent_zone(zone) else { continue };
        let Some(file) = store.stat(zone, OUTPUT_FILE).map_err(|e| format!("reading {zone}: {e}"))? else {
            continue;
        };
        let row = by_id.entry(id.to_string()).or_insert_with(|| HistoryAgent { definition_id: id.to_string(), ..Default::default() });
        let bytes = file.size.max(0) as u64;
        if archived {
            row.archives += 1;
            row.archive_bytes += bytes;
        } else {
            row.current_bytes += bytes;
        }
        row.last_active_ms = row.last_active_ms.max(file.modts.max(0) as u64);
    }
    let mut rows: Vec<HistoryAgent> = by_id.into_values().collect();
    for row in &mut rows {
        match info(&row.definition_id) {
            Some(agent) => {
                row.name = agent.name;
                row.provider = agent.provider;
                if !agent.fork_of.is_empty() {
                    row.fork_of_name = info(&agent.fork_of).map(|p| p.name).unwrap_or_default();
                    row.fork_of = agent.fork_of;
                }
            }
            None => row.deleted = true,
        }
    }
    rows.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms).then_with(|| a.definition_id.cmp(&b.definition_id)));
    Ok(rows)
}

#[cfg(test)]
#[path = "history_registry_tests.rs"]
mod tests;
