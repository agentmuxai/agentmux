// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tracking for what an agent reads as project instructions.
//!
//! Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. The
//! resolver in `backend::project_instructions` answers "what will this agent
//! read, right now". This records those answers so the next one can be
//! compared against the last, which is what makes a repository's own
//! `CLAUDE.md` changing under a running agent visible at all.
//!
//! **Hash and metadata only, never content.** `db_agent_native_memory` mirrors
//! content because it is a durable copy of files that can become unreachable;
//! these files belong to the repository, are always readable from it, and are
//! never written by AgentMux. A second copy of somebody else's file, with
//! nothing keeping it honest, would be a liability rather than a feature.

use rusqlite::params;
use serde::Serialize;

use super::error::StoreError;
use super::store::Store;

/// One recorded observation of one instruction file.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectInstructionObservation {
    pub path: String,
    pub content_hash: String,
    pub size_bytes: i64,
    /// `"agentmux"` or `"foreign"`, as the resolver classified it.
    pub owner: String,
    pub existed: bool,
    pub observed_at: i64,
}

/// How a file compares to the last time it was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionChange {
    /// No prior observation — the first time this agent was looked at.
    FirstSeen,
    Unchanged,
    /// Content differs from the last observation.
    Modified,
    /// Present now, absent before.
    Added,
    /// Absent now, present before.
    Removed,
}

impl Store {
    /// Record what was observed, replacing the previous observation.
    ///
    /// Last-write-wins per `(agent_id, path)`: this is a "what did it look
    /// like most recently" record, not a history. Version history for these
    /// files would mean storing their content, which is exactly what this
    /// table declines to do — the repository's own version control already
    /// owns that job for the repository's own files.
    pub fn project_instructions_record(
        &self,
        agent_id: &str,
        observations: &[ProjectInstructionObservation],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for obs in observations {
            tx.execute(
                "INSERT INTO db_agent_project_instructions
                    (agent_id, path, content_hash, size_bytes, owner, existed, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(agent_id, path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    size_bytes   = excluded.size_bytes,
                    owner        = excluded.owner,
                    existed      = excluded.existed,
                    observed_at  = excluded.observed_at",
                params![
                    agent_id,
                    obs.path,
                    obs.content_hash,
                    obs.size_bytes,
                    obs.owner,
                    if obs.existed { 1 } else { 0 },
                    obs.observed_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every recorded observation for `agent_id`, path-ordered.
    pub fn project_instructions_list(
        &self,
        agent_id: &str,
    ) -> Result<Vec<ProjectInstructionObservation>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT path, content_hash, size_bytes, owner, existed, observed_at
             FROM db_agent_project_instructions
             WHERE agent_id = ?1
             ORDER BY path",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok(ProjectInstructionObservation {
                    path: row.get(0)?,
                    content_hash: row.get(1)?,
                    size_bytes: row.get(2)?,
                    owner: row.get(3)?,
                    existed: row.get::<_, i64>(4)? != 0,
                    observed_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Forget everything recorded for `agent_id`.
    ///
    /// Called by `agent_def_delete`, beside the row these observations belong
    /// to. There is no foreign key to enforce it — they record files an agent
    /// *reads*, which is not a relationship SQLite can express — so without
    /// that call the rows outlive the agent and a future agent reusing the id
    /// inherits somebody else's baseline, every file reporting `unchanged`
    /// against observations never made about it (ReAgent, PR #3162).
    ///
    /// Nothing else should call it: dropping observations resets every file to
    /// `FirstSeen`, which silently discards the drift this table exists to
    /// surface.
    pub fn project_instructions_forget(&self, agent_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM db_agent_project_instructions WHERE agent_id = ?1",
            params![agent_id],
        )?)
    }
}

/// Compare a fresh observation against the recorded one.
///
/// Free function rather than a `Store` method: it is pure, and keeping it that
/// way means the comparison can be unit-tested without a database and reused
/// wherever both halves are already in hand.
pub fn classify_change(
    previous: Option<&ProjectInstructionObservation>,
    current_exists: bool,
    current_hash: &str,
) -> InstructionChange {
    let Some(prev) = previous else {
        return InstructionChange::FirstSeen;
    };
    match (prev.existed, current_exists) {
        (false, false) => InstructionChange::Unchanged,
        (false, true) => InstructionChange::Added,
        (true, false) => InstructionChange::Removed,
        (true, true) => {
            // An empty hash means the file could not be read this time — a
            // permissions change, or bytes that stopped being UTF-8. Calling
            // that "modified" would be a guess; calling it unchanged would
            // hide it. The resolver already reports the error alongside, so
            // the honest answer here is that nothing is known to have changed.
            if current_hash.is_empty() || prev.content_hash.is_empty() {
                InstructionChange::Unchanged
            } else if prev.content_hash == current_hash {
                InstructionChange::Unchanged
            } else {
                InstructionChange::Modified
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(path: &str, hash: &str, existed: bool) -> ProjectInstructionObservation {
        ProjectInstructionObservation {
            path: path.to_string(),
            content_hash: hash.to_string(),
            size_bytes: 1,
            owner: "foreign".to_string(),
            existed,
            observed_at: 1,
        }
    }

    #[test]
    fn recording_twice_replaces_rather_than_duplicates() {
        let s = Store::open_in_memory().unwrap();
        s.project_instructions_record("a1", &[obs("CLAUDE.md", "h1", true)]).unwrap();
        s.project_instructions_record("a1", &[obs("CLAUDE.md", "h2", true)]).unwrap();

        let rows = s.project_instructions_list("a1").unwrap();
        assert_eq!(rows.len(), 1, "one row per (agent, path)");
        assert_eq!(rows[0].content_hash, "h2", "latest observation wins");
    }

    #[test]
    fn observations_are_scoped_to_their_agent() {
        let s = Store::open_in_memory().unwrap();
        s.project_instructions_record("a1", &[obs("CLAUDE.md", "h1", true)]).unwrap();
        s.project_instructions_record("a2", &[obs("CLAUDE.md", "other", true)]).unwrap();
        assert_eq!(s.project_instructions_list("a1").unwrap()[0].content_hash, "h1");
        assert_eq!(s.project_instructions_list("a2").unwrap()[0].content_hash, "other");
    }

    #[test]
    fn forgetting_one_agent_leaves_the_others_alone() {
        let s = Store::open_in_memory().unwrap();
        s.project_instructions_record("a1", &[obs("CLAUDE.md", "h1", true)]).unwrap();
        s.project_instructions_record("a2", &[obs("CLAUDE.md", "h1", true)]).unwrap();
        assert_eq!(s.project_instructions_forget("a1").unwrap(), 1);
        assert!(s.project_instructions_list("a1").unwrap().is_empty());
        assert_eq!(s.project_instructions_list("a2").unwrap().len(), 1);
    }

    #[test]
    fn deleting_an_agent_forgets_its_observations() {
        // Without this the rows outlive the agent, and an agent reusing the id
        // inherits a baseline that was never about it — every file reporting
        // `unchanged` against somebody else's observations (ReAgent, #3162).
        let s = Store::open_in_memory().unwrap();
        let mut def: crate::backend::storage::AgentDefinition =
            serde_json::from_value(serde_json::json!({
                "id": "doomed",
                "slug": "doomed",
                "name": "Doomed",
                "icon": "robot",
                "provider": "claude",
                "description": "",
                "working_directory": "",
                "created_at": 1,
            }))
            .unwrap();
        s.agent_def_insert(&mut def).unwrap();
        s.project_instructions_record("doomed", &[obs("CLAUDE.md", "h1", true)]).unwrap();
        s.project_instructions_record("survivor", &[obs("CLAUDE.md", "h1", true)]).unwrap();

        assert!(s.agent_def_delete("doomed").unwrap());
        assert!(
            s.project_instructions_list("doomed").unwrap().is_empty(),
            "a deleted agent must not leave observations behind"
        );
        assert_eq!(
            s.project_instructions_list("survivor").unwrap().len(),
            1,
            "and must not take anyone else's with it"
        );
    }

    #[test]
    fn classify_covers_every_transition() {
        let present = obs("CLAUDE.md", "h1", true);
        let absent = obs("CLAUDE.md", "", false);

        assert_eq!(classify_change(None, true, "h1"), InstructionChange::FirstSeen);
        assert_eq!(classify_change(Some(&present), true, "h1"), InstructionChange::Unchanged);
        assert_eq!(classify_change(Some(&present), true, "h2"), InstructionChange::Modified);
        assert_eq!(classify_change(Some(&present), false, ""), InstructionChange::Removed);
        assert_eq!(classify_change(Some(&absent), true, "h1"), InstructionChange::Added);
        assert_eq!(classify_change(Some(&absent), false, ""), InstructionChange::Unchanged);
    }

    #[test]
    fn an_unreadable_file_is_not_reported_as_modified() {
        // The resolver leaves the hash empty when it cannot read the bytes.
        // Guessing "modified" from that would cry wolf on a permissions
        // change; the resolver reports the error separately.
        let present = obs("CLAUDE.md", "h1", true);
        assert_eq!(classify_change(Some(&present), true, ""), InstructionChange::Unchanged);

        let unreadable_before = obs("CLAUDE.md", "", true);
        assert_eq!(
            classify_change(Some(&unreadable_before), true, "h1"),
            InstructionChange::Unchanged,
            "no baseline to compare against either"
        );
    }
}
