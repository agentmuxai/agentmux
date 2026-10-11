// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Work claims: an agent saying "I'm working on this" about a path, branch
//! or topic, so others see it in `WhoIsWorkingOn` and in overlap notes
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.4).
//!
//! A claim never blocks anyone: it is information with an expiry. It lives
//! in the always-global identity store, like the work queue, so every
//! channel on this computer sees every claim. It has its own table rather
//! than being a work-queue row: a queue item whose lease lapses goes back to
//! `open`, where `WorkClaim` would hand it to another agent as an
//! instruction. An expired claim is simply gone: it is filtered out on every
//! read and deleted on the next write.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// One live claim. `repo`, `path`, `absolute_path`, `branch` and `topic` are
/// the claimed target, resolved the way `WhoIsWorkingOn` resolves a question
/// (`work_facts::matching::Target`); any of them may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkClaim {
    pub id: String,
    pub agent: String,
    /// The claimer's UID, when its request carried one; empty otherwise.
    #[serde(default)]
    pub agent_uid: String,
    pub channel: String,
    pub repo: Option<String>,
    /// Repository-relative; `""` is the whole repository.
    pub path: Option<String>,
    pub absolute_path: Option<String>,
    pub branch: Option<String>,
    pub topic: Option<String>,
    pub note: String,
    pub created_at: i64,
    pub expires_at: i64,
}

const COLS: &str =
    "id, agent, agent_uid, channel, repo, path, absolute_path, branch, topic, note, created_at, expires_at";

fn row_to_claim(row: &Row) -> rusqlite::Result<WorkClaim> {
    Ok(WorkClaim {
        id: row.get("id")?,
        agent: row.get("agent")?,
        agent_uid: row.get("agent_uid")?,
        channel: row.get("channel")?,
        repo: row.get("repo")?,
        path: row.get("path")?,
        absolute_path: row.get("absolute_path")?,
        branch: row.get("branch")?,
        topic: row.get("topic")?,
        note: row.get("note")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

/// The holder test, as the work queue's: by UID when the row and the caller
/// both have one, else by name (case-insensitively). `?1` name, `?2` UID.
const HOLDER: &str = "((agent_uid <> '' AND ?2 <> '' AND agent_uid = ?2) \
                      OR ((agent_uid = '' OR ?2 = '') AND agent = ?1 COLLATE NOCASE))";

impl Store {
    /// Record `claim`, or renew the holder's existing claim on the same
    /// target: its id and `created_at` stay, its note and expiry are
    /// replaced. Expired claims are deleted first. Returns the stored claim.
    pub fn work_claim_put(&self, claim: &WorkClaim, now_ms: i64) -> Result<WorkClaim, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_work_claims WHERE expires_at <= ?1", params![now_ms])?;
        // `IS` compares NULL to NULL as equal: an absent part of the target
        // matches an absent part.
        let sql = format!(
            "SELECT id, created_at FROM db_work_claims
              WHERE {HOLDER}
                AND repo IS ?3 AND path IS ?4 AND absolute_path IS ?5 AND branch IS ?6
                AND topic IS ?7 COLLATE NOCASE
              LIMIT 1"
        );
        let existing: Option<(String, i64)> = conn
            .query_row(
                &sql,
                params![
                    claim.agent,
                    claim.agent_uid,
                    claim.repo,
                    claim.path,
                    claim.absolute_path,
                    claim.branch,
                    claim.topic
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let mut stored = claim.clone();
        if let Some((id, created_at)) = existing {
            stored.id = id;
            stored.created_at = created_at;
        }
        conn.execute(
            &format!("INSERT OR REPLACE INTO db_work_claims ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"),
            params![
                stored.id,
                stored.agent,
                stored.agent_uid,
                stored.channel,
                stored.repo,
                stored.path,
                stored.absolute_path,
                stored.branch,
                stored.topic,
                stored.note,
                stored.created_at,
                stored.expires_at
            ],
        )?;
        Ok(stored)
    }

    /// Every unexpired claim, oldest first.
    pub fn work_claims_live(&self, now_ms: i64) -> Result<Vec<WorkClaim>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt =
            conn.prepare(&format!("SELECT {COLS} FROM db_work_claims WHERE expires_at > ?1 ORDER BY created_at, id"))?;
        let rows = stmt.query_map(params![now_ms], row_to_claim)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Release the holder's claims: the one with `id`, or all of them when
    /// `id` is `None`. Returns what was released; another agent's claim is
    /// never touched.
    pub fn work_claim_release(
        &self,
        agent: &str,
        agent_uid: &str,
        id: Option<&str>,
        now_ms: i64,
    ) -> Result<Vec<WorkClaim>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let sql = format!(
            "DELETE FROM db_work_claims
              WHERE {HOLDER} AND (?3 IS NULL OR id = ?3) AND expires_at > ?4
              RETURNING {COLS}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![agent, agent_uid, id, now_ms], row_to_claim)?;
        let released = rows.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        conn.execute("DELETE FROM db_work_claims WHERE expires_at <= ?1", params![now_ms])?;
        Ok(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000;
    const MIN: i64 = 60_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        (dir, s)
    }

    fn claim(id: &str, agent: &str, path: &str) -> WorkClaim {
        WorkClaim {
            id: id.into(),
            agent: agent.into(),
            channel: "stable".into(),
            repo: Some("o/r".into()),
            path: Some(path.into()),
            note: "presence work".into(),
            created_at: NOW,
            expires_at: NOW + 120 * MIN,
            ..Default::default()
        }
    }

    #[test]
    fn a_claim_is_listed_until_it_expires() {
        let (_d, s) = store();
        s.work_claim_put(&claim("c1", "Agent5", "crates/srv/src/muxbus"), NOW).unwrap();
        assert_eq!(s.work_claims_live(NOW).unwrap().len(), 1);
        assert_eq!(s.work_claims_live(NOW + 120 * MIN - 1).unwrap().len(), 1);
        assert!(s.work_claims_live(NOW + 120 * MIN).unwrap().is_empty(), "expired");
    }

    #[test]
    fn claiming_the_same_target_again_renews_it_and_keeps_its_id() {
        let (_d, s) = store();
        s.work_claim_put(&claim("c1", "Agent5", "a.rs"), NOW).unwrap();
        let again = WorkClaim {
            note: "still on it".into(),
            created_at: NOW + 30 * MIN,
            expires_at: NOW + 200 * MIN,
            ..claim("c2", "agent5", "a.rs")
        };
        let stored = s.work_claim_put(&again, NOW + 30 * MIN).unwrap();
        assert_eq!((stored.id.as_str(), stored.created_at), ("c1", NOW));
        let live = s.work_claims_live(NOW + 30 * MIN).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!((live[0].note.as_str(), live[0].expires_at), ("still on it", NOW + 200 * MIN));

        // Another target, or another agent on the same one, is a second claim.
        s.work_claim_put(&claim("c3", "Agent5", "b.rs"), NOW).unwrap();
        s.work_claim_put(&claim("c4", "Agent4", "a.rs"), NOW).unwrap();
        assert_eq!(s.work_claims_live(NOW).unwrap().len(), 3);
    }

    #[test]
    fn only_the_holder_releases_one_claim_or_all_of_its_own() {
        let (_d, s) = store();
        s.work_claim_put(&claim("c1", "Agent5", "a.rs"), NOW).unwrap();
        s.work_claim_put(&claim("c2", "Agent5", "b.rs"), NOW).unwrap();
        s.work_claim_put(&claim("c3", "Agent4", "a.rs"), NOW).unwrap();

        assert!(s.work_claim_release("Agent4", "", Some("c1"), NOW).unwrap().is_empty(), "not Agent4's");
        let one = s.work_claim_release("agent5", "", Some("c1"), NOW).unwrap();
        assert_eq!(one.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["c1"]);
        let rest = s.work_claim_release("Agent5", "", None, NOW).unwrap();
        assert_eq!(rest.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["c2"]);
        let left = s.work_claims_live(NOW).unwrap();
        assert_eq!(left.iter().map(|c| c.agent.as_str()).collect::<Vec<_>>(), vec!["Agent4"]);
    }

    #[test]
    fn a_uid_decides_when_both_sides_have_one() {
        let (_d, s) = store();
        let mine = WorkClaim { agent_uid: "u1".into(), ..claim("c1", "Agent5", "a.rs") };
        s.work_claim_put(&mine, NOW).unwrap();
        assert!(s.work_claim_release("Agent5", "u2", None, NOW).unwrap().is_empty(), "same name, another UID");
        assert_eq!(s.work_claim_release("Agent5", "u1", None, NOW).unwrap().len(), 1);
    }
}
