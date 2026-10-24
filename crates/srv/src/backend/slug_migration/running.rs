// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether an agent runs anywhere on this host, from what every instance
//! leaves on disk: its lease (a pane holds one for its process's life, a
//! turn for the turn), its registrations in the cross-channel registry, and
//! each version's own registrations. Read from the files, never by asking an
//! instance, so a channel that doesn't answer is still seen.

use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use super::home::{fold, open_ro, place, Home, Install};
use super::plan::Def;
use crate::backend::reactive::registry::{pid_alive, AgentEntry};
use crate::backend::rpc_types::ForkSlugRunning;
use agentmux_common::slug::{agent_route_name, file_stem};

/// A registration whose srv is alive, from `<dir>/<name>.json`.
pub(super) fn registration(dir: &Path, name: &str) -> Option<AgentEntry> {
    let entry: AgentEntry = serde_json::from_slice(&std::fs::read(dir.join(format!("{}.json", file_stem(name)))).ok()?).ok()?;
    pid_alive(entry.pid).then_some(entry)
}

/// The live entries filed under `name` in the cross-channel registry.
pub(super) fn shared_entries(home: &Home, name: &str) -> Vec<AgentEntry> {
    let dir = home.reactive().join(file_stem(name));
    let Ok(files) = std::fs::read_dir(dir) else { return Vec::new() };
    files
        .flatten()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|f| serde_json::from_slice::<AgentEntry>(&std::fs::read(f.path()).ok()?).ok())
        .filter(|e| pid_alive(e.pid))
        .collect()
}

/// The agent uid a block belongs to (`agentId`, else the legacy
/// `agent:id`), from that version's own `objects.db`.
pub(super) fn block_agent(conn: &Connection, block_id: &str) -> Option<String> {
    if block_id.is_empty() {
        return None;
    }
    let data: Vec<u8> = conn.query_row("SELECT data FROM db_block WHERE oid = ?1", [block_id], |r| r.get(0)).ok()?;
    let block: Value = serde_json::from_slice(&data).ok()?;
    let meta = block.get("meta")?;
    ["agentId", "agent:id"].iter().find_map(|k| meta.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string))
}

/// The names an agent may be registered under: its slug, its former slugs
/// and its display name's route name.
fn names_of(def: &Def) -> Vec<String> {
    let mut names = vec![fold(&def.slug), fold(&agent_route_name(&def.name))];
    names.extend(def.former.iter().map(|f| fold(f)));
    names.sort();
    names.dedup();
    names.retain(|n| !n.is_empty());
    names
}

/// Where `def` runs now. A registration under one of its names counts unless
/// it is provably another agent's pane; a lease counts while it is fresh and
/// its holder isn't provably gone.
pub(super) fn running(home: &Home, installs: &[Install], def: &Def) -> Result<Vec<ForkSlugRunning>, String> {
    let mut out = Vec::new();
    if let Some(lease) = live_lease(home, &def.uid) {
        out.push(lease);
    }
    let names = names_of(def);
    for name in &names {
        for e in shared_entries(home, name) {
            if e.uid.is_empty() || e.uid == def.uid {
                out.push(ForkSlugRunning { block_id: e.block_id, channel: e.channel, how: format!("registered as {name}") });
            }
        }
    }
    for install in installs {
        for version in &install.versions {
            let mut conn: Option<Connection> = None;
            for name in &names {
                let Some(e) = registration(&version.registrations(), name) else { continue };
                if conn.is_none() {
                    conn = Some(open_ro(&version.objects_db())?);
                }
                let owner = conn.as_ref().and_then(|c| block_agent(c, &e.block_id));
                if owner.is_none() || owner.as_deref() == Some(def.uid.as_str()) {
                    out.push(ForkSlugRunning {
                        block_id: e.block_id,
                        channel: place(install, version),
                        how: format!("registered as {name}"),
                    });
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| seen.insert((r.block_id.clone(), r.channel.clone(), r.how.clone())));
    Ok(out)
}

/// The agent's lease, if live: renewed within its TTL, and its holder not
/// provably gone (recorded on this host with a pid that is no longer
/// running). An unreadable file that is still fresh counts, as the lease
/// store itself fails closed.
fn live_lease(home: &Home, uid: &str) -> Option<ForkSlugRunning> {
    let path = home.leases().join(format!("{uid}.lease.json"));
    let bytes = std::fs::read(&path).ok()?;
    let ttl = crate::registry::LEASE_TTL_MS as i64;
    let now = agentmux_common::time::now_ms();
    let Ok(lease) = serde_json::from_slice::<Value>(&bytes) else {
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?.elapsed().ok()?.as_millis() as i64;
        return (age <= ttl).then(|| ForkSlugRunning { block_id: String::new(), channel: String::new(), how: "an unreadable lease".into() });
    };
    let num = |k: &str| lease.get(k).and_then(Value::as_i64).unwrap_or(0);
    let text = |k: &str| lease.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    if now - num("renewed_at_ms") > ttl {
        return None;
    }
    let host = text("hostname");
    let this_host = whoami::fallible::hostname().unwrap_or_default().trim().to_lowercase();
    let pid = num("pid") as u32;
    if !host.is_empty() && host == this_host && pid != 0 && !pid_alive(pid) {
        return None;
    }
    Some(ForkSlugRunning { block_id: text("block_id"), channel: text("channel"), how: "holds its lease".into() })
}
