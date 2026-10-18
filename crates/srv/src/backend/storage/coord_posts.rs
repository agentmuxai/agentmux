// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Coordination posts: what an agent published to a repository's channel for
//! any agent working on the same thing to read (`PostToChannel`,
//! `ReadChannel`, `WhoIsWorkingOn`'s `recent_posts`), and the claims and
//! releases recorded there (docs/specs/SPEC_AGENT_COORDINATION_CHANNEL_2026_10_15.md).
//!
//! Like work claims, they live in the always-global identity store, so every
//! channel on this computer sees every post. A post is an explicit
//! publication, never a copy of a direct message. Posts expire after
//! [`RETENTION_MS`], and a repository keeps at most [`MAX_PER_REPO`]; both are
//! enforced on every write.

use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// How long a post is kept.
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// The most posts one repository keeps; the oldest go first.
pub const MAX_PER_REPO: i64 = 500;

/// One post. `repo` is the channel; `path`, `branch`, `pr` and `topic` say
/// what in it the post is about, any of them absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordPost {
    pub id: String,
    pub repo: String,
    /// Repository-relative; `""` is the whole repository.
    pub path: Option<String>,
    pub branch: Option<String>,
    pub pr: Option<i64>,
    pub topic: Option<String>,
    /// `post`, `claim` or `release`.
    pub kind: String,
    pub author: String,
    #[serde(default)]
    pub author_uid: String,
    pub author_channel: String,
    /// srv identified the author by its agent token.
    pub identified: bool,
    pub body: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// The table, created on every open of the identity store, with no schema
/// version bump (see `work_claims::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_coord_posts (
        id             TEXT PRIMARY KEY,
        repo           TEXT NOT NULL,
        path           TEXT,
        branch         TEXT,
        pr             INTEGER,
        topic          TEXT,
        kind           TEXT NOT NULL DEFAULT 'post',
        author         TEXT NOT NULL,
        author_uid     TEXT NOT NULL DEFAULT '',
        author_channel TEXT NOT NULL DEFAULT '',
        identified     INTEGER NOT NULL DEFAULT 0,
        body           TEXT NOT NULL,
        created_at     INTEGER NOT NULL,
        expires_at     INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_ids_coord_posts_repo ON db_coord_posts(repo, created_at);
    CREATE INDEX IF NOT EXISTS idx_ids_coord_posts_expires ON db_coord_posts(expires_at);";

const COLS: &str =
    "id, repo, path, branch, pr, topic, kind, author, author_uid, author_channel, identified, body, created_at, expires_at";

fn row_to_post(row: &Row) -> rusqlite::Result<CoordPost> {
    Ok(CoordPost {
        id: row.get("id")?,
        repo: row.get("repo")?,
        path: row.get("path")?,
        branch: row.get("branch")?,
        pr: row.get("pr")?,
        topic: row.get("topic")?,
        kind: row.get("kind")?,
        author: row.get("author")?,
        author_uid: row.get("author_uid")?,
        author_channel: row.get("author_channel")?,
        identified: row.get::<_, i64>("identified")? != 0,
        body: row.get("body")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

impl Store {
    /// Record `post`. Expired posts are deleted first, and the repository is
    /// trimmed to its newest [`MAX_PER_REPO`] after.
    pub fn coord_post_put(&self, post: &CoordPost, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_coord_posts WHERE expires_at <= ?1", params![now_ms])?;
        conn.execute(
            &format!("INSERT OR REPLACE INTO db_coord_posts ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"),
            params![
                post.id,
                post.repo,
                post.path,
                post.branch,
                post.pr,
                post.topic,
                post.kind,
                post.author,
                post.author_uid,
                post.author_channel,
                post.identified as i64,
                post.body,
                post.created_at,
                post.expires_at
            ],
        )?;
        conn.execute(
            "DELETE FROM db_coord_posts WHERE repo = ?1 AND id NOT IN
               (SELECT id FROM db_coord_posts WHERE repo = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
            params![post.repo, MAX_PER_REPO],
        )?;
        Ok(())
    }

    /// `repo`'s unexpired posts created at or after `since_ms`, newest first;
    /// posts of the same millisecond newest-written first (`rowid`: the ids are
    /// random, so they say nothing of order).
    /// The finer filters (path, branch, pr, words) are the caller's
    /// (`work_facts::coord`): a repository holds a bounded number.
    pub fn coord_posts_in(&self, repo: &str, since_ms: i64, now_ms: i64) -> Result<Vec<CoordPost>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_coord_posts
              WHERE repo = ?1 AND created_at >= ?2 AND expires_at > ?3
              ORDER BY created_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map(params![repo, since_ms, now_ms], row_to_post)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
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

    fn post(id: &str, repo: &str, at: i64) -> CoordPost {
        CoordPost {
            id: id.into(),
            repo: repo.into(),
            path: Some("crates/x.rs".into()),
            kind: "post".into(),
            author: "Camper".into(),
            author_channel: "stable".into(),
            identified: true,
            body: format!("note {id}"),
            created_at: at,
            expires_at: at + RETENTION_MS,
            ..Default::default()
        }
    }

    #[test]
    fn stores_and_reads_back_newest_first_per_repo() {
        let (_d, s) = store();
        s.coord_post_put(&post("a", "o/r", NOW - 2 * MIN), NOW).unwrap();
        s.coord_post_put(&post("b", "o/r", NOW - MIN), NOW).unwrap();
        s.coord_post_put(&post("c", "o/other", NOW - MIN), NOW).unwrap();
        let got = s.coord_posts_in("o/r", 0, NOW).unwrap();
        assert_eq!(got.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        assert_eq!(got[0], post("b", "o/r", NOW - MIN));
    }

    /// Posts made in the same millisecond (a claim and its release, say) read
    /// in the order they were written, whatever their random ids sort as.
    #[test]
    fn posts_made_in_the_same_millisecond_read_newest_written_first() {
        let (_d, s) = store();
        // Ids that sort against the order they were written in, so an id tiebreak would get it wrong.
        s.coord_post_put(&post("z", "o/r", NOW - MIN), NOW).unwrap();
        s.coord_post_put(&post("m", "o/r", NOW - MIN), NOW).unwrap();
        s.coord_post_put(&post("a", "o/r", NOW - MIN), NOW).unwrap();
        let got = s.coord_posts_in("o/r", 0, NOW).unwrap();
        assert_eq!(got.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["a", "m", "z"]);
    }

    #[test]
    fn since_and_expiry_filter_reads() {
        let (_d, s) = store();
        s.coord_post_put(&post("old", "o/r", NOW - 10 * MIN), NOW).unwrap();
        s.coord_post_put(&post("new", "o/r", NOW - MIN), NOW).unwrap();
        let recent = s.coord_posts_in("o/r", NOW - 5 * MIN, NOW).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, "new");
        // Past the retention, nothing is read, and the next write deletes it.
        let later = NOW + RETENTION_MS;
        assert!(s.coord_posts_in("o/r", 0, later).unwrap().is_empty());
        s.coord_post_put(&post("next", "o/r", later), later).unwrap();
        let count: i64 = s
            .conn()
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM db_coord_posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_repository_keeps_its_newest_posts_up_to_the_cap() {
        let (_d, s) = store();
        for i in 0..(MAX_PER_REPO + 5) {
            s.coord_post_put(&post(&format!("p{i:04}"), "o/r", NOW - (MAX_PER_REPO + 5 - i) * 1000), NOW).unwrap();
        }
        s.coord_post_put(&post("elsewhere", "o/other", NOW - MIN), NOW).unwrap();
        let got = s.coord_posts_in("o/r", 0, NOW).unwrap();
        assert_eq!(got.len() as i64, MAX_PER_REPO);
        assert_eq!(got.last().unwrap().id, "p0005"); // the five oldest went
        assert_eq!(s.coord_posts_in("o/other", 0, NOW).unwrap().len(), 1);
    }
}
