// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! On-demand, once-per-definition activity summary — see
//! `db_agent_activity_summaries` in migrations.rs (`OBJECT_SCHEMA_VERSION`
//! v27) and
//! docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §5a.
//!
//! Fallback preview text for the AgentPicker's "My Agents" rows whose
//! instance has no structured `output.state.json` conversation snapshot —
//! generated once, lazily, via `app_api::session::generate_definition_activity_summary`,
//! and cached here forever (never regenerated once non-empty). An absent
//! row IS the "not generated yet" state; there is no separate pending flag.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentActivitySummary {
    pub definition_id: String,
    pub summary: String,
    pub updated_at: i64,
}

impl Store {
    pub fn agent_activity_summary_get(
        &self,
        definition_id: &str,
    ) -> Result<Option<AgentActivitySummary>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT definition_id, summary, updated_at FROM db_agent_activity_summaries WHERE definition_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![definition_id], map_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Upsert — a definition's summary is generated at most once in the
    /// common case, but an upsert (rather than plain INSERT) keeps this
    /// safe against the rare double-generation race the Ambient Model Call
    /// gateway doesn't fully close (two independent process starts, or a
    /// restart mid-generation).
    pub fn agent_activity_summary_set(
        &self,
        definition_id: &str,
        summary: &str,
        updated_at: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_activity_summaries (definition_id, summary, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(definition_id) DO UPDATE SET summary = ?2, updated_at = ?3",
            params![definition_id, summary, updated_at],
        )?;
        Ok(())
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentActivitySummary> {
    Ok(AgentActivitySummary {
        definition_id: row.get(0)?,
        summary: row.get(1)?,
        updated_at: row.get(2)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open(tmp.path()).unwrap()
    }

    #[test]
    fn get_returns_none_when_never_generated() {
        let store = object_store();
        assert!(store.agent_activity_summary_get("def-1").unwrap().is_none());
    }

    #[test]
    fn set_then_get_roundtrips() {
        let store = object_store();
        store.agent_activity_summary_set("def-1", "Debugging a flaky test", 1000).unwrap();
        let got = store.agent_activity_summary_get("def-1").unwrap().unwrap();
        assert_eq!(got.definition_id, "def-1");
        assert_eq!(got.summary, "Debugging a flaky test");
        assert_eq!(got.updated_at, 1000);
    }

    #[test]
    fn set_twice_upserts_instead_of_erroring() {
        let store = object_store();
        store.agent_activity_summary_set("def-1", "first summary", 1000).unwrap();
        store.agent_activity_summary_set("def-1", "second summary", 2000).unwrap();
        let got = store.agent_activity_summary_get("def-1").unwrap().unwrap();
        assert_eq!(got.summary, "second summary");
        assert_eq!(got.updated_at, 2000);
    }

    #[test]
    fn distinct_definitions_do_not_collide() {
        let store = object_store();
        store.agent_activity_summary_set("def-1", "summary one", 1000).unwrap();
        store.agent_activity_summary_set("def-2", "summary two", 2000).unwrap();
        assert_eq!(store.agent_activity_summary_get("def-1").unwrap().unwrap().summary, "summary one");
        assert_eq!(store.agent_activity_summary_get("def-2").unwrap().unwrap().summary, "summary two");
    }
}
