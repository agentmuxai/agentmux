// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The bulletin board: short posts an agent sees at its next real turn, never waking one
//! (`docs/specs/SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §4). A post lives here so an agent that is stopped
//! or starts later still reads what it missed, and a restart of srv loses nothing. Each reader has a "seen" set,
//! so a bulletin is shown to an agent once.
//!
//! Like coordination posts, they live in the always-global identity store, and the table is created on every
//! open of it with no schema version bump (see `work_claims::SCHEMA`).

use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// One bulletin. `key` (may be empty) names a standing post: a later post under the same key replaces it.
/// `targets` are agent route names, lower-case; empty means every agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bulletin {
    pub id: String,
    pub key: String,
    pub author: String,
    pub author_uid: String,
    pub body: String,
    pub targets: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

/// The tables, created on every open of the identity store.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_bulletins (
        id         TEXT PRIMARY KEY,
        topic      TEXT NOT NULL DEFAULT '',
        author     TEXT NOT NULL,
        author_uid TEXT NOT NULL DEFAULT '',
        body       TEXT NOT NULL,
        targets    TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_ids_bulletins_expires ON db_bulletins(expires_at);
    CREATE TABLE IF NOT EXISTS db_bulletin_log (
        author TEXT NOT NULL,
        at     INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_ids_bulletin_log_at ON db_bulletin_log(at);
    CREATE TABLE IF NOT EXISTS db_bulletin_seen (
        reader      TEXT NOT NULL,
        bulletin_id TEXT NOT NULL,
        PRIMARY KEY (reader, bulletin_id)
    );";

/// How long a post stays in the log the hourly limit counts.
const LOG_KEPT_MS: i64 = 2 * 60 * 60 * 1000;

const COLS: &str = "id, topic, author, author_uid, body, targets, created_at, expires_at";

fn row_to_bulletin(row: &Row) -> rusqlite::Result<Bulletin> {
    let targets: String = row.get("targets")?;
    Ok(Bulletin {
        id: row.get("id")?,
        key: row.get("topic")?,
        author: row.get("author")?,
        author_uid: row.get("author_uid")?,
        body: row.get("body")?,
        targets: targets.split(',').filter(|t| !t.is_empty()).map(str::to_string).collect(),
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

impl Bulletin {
    /// Whether `reader` (a route name, lower-case) is among the targets, or there are none.
    pub fn is_for(&self, reader: &str) -> bool {
        self.targets.is_empty() || self.targets.iter().any(|t| t == reader)
    }
}

impl Store {
    /// Record `b`: expired bulletins go first, and so does the same author's earlier one under the same
    /// non-empty key (another author's is not touched). The post is also logged, for [`Self::bulletin_counts`]:
    /// the log outlives a replacement or an expiry, so the hourly limit can't be dodged by either.
    pub fn bulletin_put(&self, b: &Bulletin, now_ms: i64) -> Result<(), StoreError> {
        self.bulletin_put_replacing(b, now_ms, false)
    }

    /// [`Self::bulletin_put`] for a notice that is one for the whole fleet: every author's earlier post under
    /// the same key goes, not only `b`'s author's.
    pub fn bulletin_put_for_fleet(&self, b: &Bulletin, now_ms: i64) -> Result<(), StoreError> {
        self.bulletin_put_replacing(b, now_ms, true)
    }

    fn bulletin_put_replacing(&self, b: &Bulletin, now_ms: i64, any_author: bool) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "DELETE FROM db_bulletins WHERE expires_at <= ?1
               OR (?2 <> '' AND topic = ?2 AND (?4 OR author = ?3 COLLATE NOCASE))",
            params![now_ms, b.key, b.author, any_author],
        )?;
        conn.execute("DELETE FROM db_bulletin_log WHERE at < ?1", params![now_ms - LOG_KEPT_MS])?;
        conn.execute("INSERT INTO db_bulletin_log (author, at) VALUES (?1, ?2)", params![b.author.to_lowercase(), b.created_at])?;
        conn.execute("DELETE FROM db_bulletin_seen WHERE bulletin_id NOT IN (SELECT id FROM db_bulletins)", [])?;
        conn.execute(
            &format!("INSERT INTO db_bulletins ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"),
            params![b.id, b.key, b.author, b.author_uid, b.body, b.targets.join(","), b.created_at, b.expires_at],
        )?;
        Ok(())
    }

    /// How many bulletins `author` posted since `since_ms`, and how many of theirs are live.
    pub fn bulletin_counts(&self, author: &str, since_ms: i64, now_ms: i64) -> Result<(i64, i64), StoreError> {
        let conn = self.conn().lock().unwrap();
        let recent = conn.query_row(
            "SELECT COUNT(*) FROM db_bulletin_log WHERE author = ?1 AND at >= ?2",
            params![author.to_lowercase(), since_ms],
            |r| r.get(0),
        )?;
        let live = conn.query_row(
            "SELECT COUNT(*) FROM db_bulletins WHERE author = ?1 COLLATE NOCASE AND expires_at > ?2",
            params![author, now_ms],
            |r| r.get(0),
        )?;
        Ok((recent, live))
    }

    /// The live bulletins `reader` (a route name, lower-case) has not seen, oldest first: for it, and not its own.
    pub fn bulletins_unseen(&self, reader: &str, now_ms: i64) -> Result<Vec<Bulletin>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_bulletins
              WHERE expires_at > ?1 AND author <> ?2 COLLATE NOCASE
                AND id NOT IN (SELECT bulletin_id FROM db_bulletin_seen WHERE reader = ?2)
              ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![now_ms, reader], row_to_bulletin)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?.into_iter().filter(|b| b.is_for(reader)).collect())
    }

    /// `reader` has seen `ids`.
    pub fn bulletins_mark_seen(&self, reader: &str, ids: &[String]) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        for id in ids {
            conn.execute("INSERT OR IGNORE INTO db_bulletin_seen (reader, bulletin_id) VALUES (?1, ?2)", params![reader, id])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000_000;
    const MIN: i64 = 60_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        (dir, s)
    }

    fn bulletin(id: &str, key: &str, author: &str, targets: &[&str], at: i64) -> Bulletin {
        Bulletin {
            id: id.into(),
            key: key.into(),
            author: author.into(),
            body: format!("text {id}"),
            targets: targets.iter().map(|t| t.to_string()).collect(),
            created_at: at,
            expires_at: at + 60 * MIN,
            ..Default::default()
        }
    }

    #[test]
    fn a_reader_sees_each_bulletin_for_it_once_and_never_its_own() {
        let (_d, s) = store();
        s.bulletin_put(&bulletin("a", "", "AgentX", &[], NOW - 2 * MIN), NOW).unwrap();
        s.bulletin_put(&bulletin("b", "", "AgentY", &["agent1"], NOW - MIN), NOW).unwrap();
        s.bulletin_put(&bulletin("c", "", "AgentY", &["camper"], NOW - MIN), NOW).unwrap();
        let got = s.bulletins_unseen("agent1", NOW).unwrap();
        assert_eq!(got.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"], "oldest first, targets honoured");
        s.bulletins_mark_seen("agent1", &["a".into(), "b".into()]).unwrap();
        assert!(s.bulletins_unseen("agent1", NOW).unwrap().is_empty(), "shown once");
        assert_eq!(s.bulletins_unseen("camper", NOW).unwrap().len(), 2, "another reader still sees them");
        assert!(s.bulletins_unseen("agentx", NOW).unwrap().iter().all(|b| b.author != "AgentX"), "not its own");
    }

    #[test]
    fn a_later_post_under_the_same_key_replaces_the_earlier_and_is_shown_again() {
        let (_d, s) = store();
        s.bulletin_put(&bulletin("m1", "mode", "AgentY", &[], NOW - 2 * MIN), NOW).unwrap();
        s.bulletins_mark_seen("agent1", &["m1".into()]).unwrap();
        s.bulletin_put(&bulletin("m2", "mode", "AgentY", &[], NOW - MIN), NOW).unwrap();
        let got = s.bulletins_unseen("agent1", NOW).unwrap();
        assert_eq!(got.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["m2"]);
    }

    #[test]
    fn the_fleets_notice_replaces_every_authors_earlier_one_and_an_ordinary_post_only_its_own() {
        let (_d, s) = store();
        s.bulletin_put_for_fleet(&bulletin("m1", "mode", "AgentY", &[], NOW - 3 * MIN), NOW).unwrap();
        s.bulletin_put_for_fleet(&bulletin("m2", "mode", "AgentX", &[], NOW - 2 * MIN), NOW).unwrap();
        let got = s.bulletins_unseen("agent1", NOW).unwrap();
        assert_eq!(got.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["m2"], "the last setter's notice is the only one");
        s.bulletin_put(&bulletin("s1", "status", "AgentY", &[], NOW - MIN), NOW).unwrap();
        s.bulletin_put(&bulletin("s2", "status", "AgentX", &[], NOW), NOW).unwrap();
        let ids: Vec<String> = s.bulletins_unseen("agent1", NOW).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["m2", "s1", "s2"], "an ordinary key is still per author");
    }

    #[test]
    fn expired_bulletins_are_not_read_and_the_next_write_deletes_them() {
        let (_d, s) = store();
        s.bulletin_put(&bulletin("old", "", "AgentY", &[], NOW), NOW).unwrap();
        let later = NOW + 61 * MIN;
        assert!(s.bulletins_unseen("agent1", later).unwrap().is_empty());
        s.bulletin_put(&bulletin("new", "", "AgentY", &[], later), later).unwrap();
        let count: i64 = s.conn().lock().unwrap().query_row("SELECT COUNT(*) FROM db_bulletins", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn replacing_or_expiring_a_post_does_not_reset_the_hourly_count() {
        let (_d, s) = store();
        for i in 0..3 {
            let b = Bulletin { expires_at: NOW - 5 * MIN + i * 1000 + 1_000, ..bulletin(&format!("m{i}"), "mode", "AgentY", &[], NOW - 5 * MIN + i * 1000) };
            s.bulletin_put(&b, NOW - 5 * MIN + i * 1000).unwrap();
        }
        assert_eq!(s.bulletin_counts("AgentY", NOW - 60 * MIN, NOW).unwrap().0, 3, "three posts under one key still count three");
    }

    #[test]
    fn a_key_is_replaced_only_by_the_same_author() {
        let (_d, s) = store();
        s.bulletin_put(&bulletin("x1", "mode", "AgentX", &[], NOW - 2 * MIN), NOW).unwrap();
        s.bulletin_put(&bulletin("y1", "mode", "AgentY", &[], NOW - MIN), NOW).unwrap();
        let ids: Vec<String> = s.bulletins_unseen("agent1", NOW).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["x1", "y1"], "AgentY's post under the key did not remove AgentX's");
        s.bulletin_put(&bulletin("y2", "mode", "agenty", &[], NOW), NOW).unwrap();
        let ids: Vec<String> = s.bulletins_unseen("agent1", NOW).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["x1", "y2"], "its own earlier one was replaced");
    }

    #[test]
    fn counts_are_an_authors_recent_and_live_posts() {
        let (_d, s) = store();
        s.bulletin_put(&bulletin("a", "", "AgentY", &[], NOW - 90 * MIN), NOW - 90 * MIN).unwrap();
        s.bulletin_put(&bulletin("b", "", "agenty", &[], NOW - 10 * MIN), NOW - 10 * MIN).unwrap();
        s.bulletin_put(&bulletin("c", "", "AgentX", &[], NOW - MIN), NOW - MIN).unwrap();
        assert_eq!(s.bulletin_counts("AgentY", NOW - 60 * MIN, NOW).unwrap(), (1, 1), "one an hour ago, one still live");
    }
}
