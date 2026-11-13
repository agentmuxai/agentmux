// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Graph versions: saving a drone makes a numbered version, a job pins one,
//! and a running run keeps the one it started on
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3 "Graph versions").
//!
//! **Module C of the engine.** The types, [`graph_hash`] and the store trait
//! are fixed here; the table and the store methods are the module's job. Its
//! acceptance tests are `versions_acceptance_tests.rs`.
//!
//! # Storage
//!
//! Versions live beside the definitions, in the shared store
//! (`state.id_store`), in a table `db_drone_versions`:
//! `(drone_id, version, graph, hash, saved_at, saved_by)`, primary key
//! `(drone_id, version)`. **No foreign key to `db_drone_definitions`** (see
//! `migrations/m0038_drop_drone_runs_definition_fk.rs`: the stores of one drone
//! are not always in one file). Deleting a drone deletes its versions
//! (`deletedrone`), the way it deletes its runs.
//!
//! The migration is **`m0049_drone_versions`** (the next number on main when
//! this was written; take the next free one if it is taken). It is not
//! registered by the interfaces PR, so that a release between the two PRs
//! cannot record it as applied while empty.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::types::DroneGraph;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::Store;

/// One saved version of a drone's graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphVersion {
    pub drone_id: String,
    /// 1 for the first save of a drone, then one more each time, per drone.
    pub version: i64,
    pub graph: DroneGraph,
    /// [`graph_hash`] of `graph`.
    pub hash: String,
    pub saved_at: i64,
    /// Who saved it: an agent id, or `operator`.
    pub saved_by: String,
}

/// Which version a job runs: the newest at the time it fires, or one number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pin {
    Latest,
    Number(i64),
}

/// The SHA-256 of a graph, as lowercase hex, of its canonical JSON: object keys
/// sorted, no whitespace. Two graphs with the same content have the same hash
/// however their JSON was ordered.
pub fn graph_hash(graph: &DroneGraph) -> String {
    let value = serde_json::to_value(graph).unwrap_or(serde_json::Value::Null);
    let mut canonical = String::new();
    write_canonical(&value, &mut canonical);
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

fn write_canonical(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&map[k], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

pub trait DroneVersionStore {
    /// Save `graph` as the next version of `drone_id`; the answer has its number and hash.
    ///
    /// **Module C.**
    fn version_save(&self, drone_id: &str, graph: &DroneGraph, saved_by: &str, now_ms: i64) -> Result<GraphVersion, StoreError>;

    /// One version, or `None` when the drone has no such number. A saved version never changes.
    ///
    /// **Module C.**
    fn version_get(&self, drone_id: &str, version: i64) -> Result<Option<GraphVersion>, StoreError>;

    /// The version `pin` names now: the newest for `Latest`, the number for `Number`; `None` when there is none.
    ///
    /// **Module C.**
    fn version_resolve(&self, drone_id: &str, pin: Pin) -> Result<Option<GraphVersion>, StoreError>;

    /// A drone's versions, newest first, at most `limit` (clamped to 1..=200).
    ///
    /// **Module C.**
    fn version_list(&self, drone_id: &str, limit: i64) -> Result<Vec<GraphVersion>, StoreError>;

    /// Delete every version of `drone_id` (the drone was deleted). Returns how many.
    ///
    /// **Module C.**
    fn versions_delete_for(&self, drone_id: &str) -> Result<usize, StoreError>;
}

/// One row as the table stores it. The graph text can be from a newer build,
/// so a parse failure is a store error, never a panic.
fn version_from_row(row: &rusqlite::Row<'_>) -> Result<GraphVersion, StoreError> {
    let graph_text: String = row.get(2)?;
    Ok(GraphVersion {
        drone_id: row.get(0)?,
        version: row.get(1)?,
        graph: serde_json::from_str(&graph_text)?,
        hash: row.get(3)?,
        saved_at: row.get(4)?,
        saved_by: row.get(5)?,
    })
}

/// The columns every read below selects, in [`version_from_row`]'s order.
const SELECT: &str = "SELECT drone_id, version, graph, hash, saved_at, saved_by FROM db_drone_versions";

impl DroneVersionStore for Store {
    fn version_save(&self, drone_id: &str, graph: &DroneGraph, saved_by: &str, now_ms: i64) -> Result<GraphVersion, StoreError> {
        let graph_text = serde_json::to_string(graph)?;
        let hash = graph_hash(graph);
        let mut conn = self.conn().lock().unwrap();
        // The number and the insert are one transaction: a second save cannot
        // read the same maximum, and the primary key is the backstop if one
        // somehow does.
        let tx = conn.transaction()?;
        let next: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM db_drone_versions WHERE drone_id = ?1",
            params![drone_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO db_drone_versions (drone_id, version, graph, hash, saved_at, saved_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![drone_id, next, graph_text, hash, now_ms, saved_by],
        )?;
        tx.commit()?;
        Ok(GraphVersion {
            drone_id: drone_id.to_string(),
            version: next,
            graph: graph.clone(),
            hash,
            saved_at: now_ms,
            saved_by: saved_by.to_string(),
        })
    }

    fn version_get(&self, drone_id: &str, version: i64) -> Result<Option<GraphVersion>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE drone_id = ?1 AND version = ?2"))?;
        let mut rows = stmt.query(params![drone_id, version])?;
        match rows.next()? {
            Some(row) => Ok(Some(version_from_row(row)?)),
            None => Ok(None),
        }
    }

    fn version_resolve(&self, drone_id: &str, pin: Pin) -> Result<Option<GraphVersion>, StoreError> {
        if let Pin::Number(version) = pin {
            return self.version_get(drone_id, version);
        }
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE drone_id = ?1 ORDER BY version DESC LIMIT 1"))?;
        let mut rows = stmt.query(params![drone_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(version_from_row(row)?)),
            None => Ok(None),
        }
    }

    fn version_list(&self, drone_id: &str, limit: i64) -> Result<Vec<GraphVersion>, StoreError> {
        let limit = limit.clamp(1, 200);
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE drone_id = ?1 ORDER BY version DESC LIMIT ?2"))?;
        let mut rows = stmt.query(params![drone_id, limit])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(version_from_row(row)?);
        }
        Ok(out)
    }

    fn versions_delete_for(&self, drone_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_drone_versions WHERE drone_id = ?1", params![drone_id])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::{FlowNode, NodePosition};
    use serde_json::json;

    fn graph(template: &str) -> DroneGraph {
        DroneGraph {
            nodes: vec![FlowNode {
                id: "r1".into(),
                position: NodePosition::default(),
                data: json!({ "kind": "response", "template": template }),
                node_type: None,
            }],
            edges: vec![],
        }
    }

    #[test]
    fn a_graphs_hash_follows_its_content() {
        assert_eq!(graph_hash(&graph("a")), graph_hash(&graph("a")));
        assert_ne!(graph_hash(&graph("a")), graph_hash(&graph("b")));
        assert_eq!(graph_hash(&graph("a")).len(), 64);
    }

    #[test]
    fn the_hash_ignores_the_order_of_keys() {
        let mut a = graph("a");
        let mut b = graph("a");
        a.nodes[0].data = json!({ "kind": "response", "template": "a" });
        b.nodes[0].data = json!({ "template": "a", "kind": "response" });
        assert_eq!(graph_hash(&a), graph_hash(&b));
    }

    #[test]
    fn a_pin_is_written_as_latest_or_a_number() {
        assert_eq!(serde_json::to_value(Pin::Latest).unwrap(), json!("latest"));
        assert_eq!(serde_json::to_value(Pin::Number(3)).unwrap(), json!({ "number": 3 }));
    }
}
