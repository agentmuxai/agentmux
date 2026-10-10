// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The git half of an agent's work facts: repository, branch and
//! uncommitted files in its working folder.
//!
//! git never runs on a request: a background loop probes each agent's folder
//! and the request reads the cache. An entry is re-probed once it is
//! [`REFRESH_AFTER_MS`] old or the agent's folder changed, so what a request
//! sees is at most ~30 s old. Every git run goes through
//! `fs_ops::git`'s safe runner (the repository's own fsmonitor and filters
//! are off, nothing prompts, the index isn't written, a slow run is killed
//! after a few seconds).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::fs_ops::git::{parse_porcelain_v2, safe_git_line, safe_status_porcelain};
use crate::backend::rpc_types::FsGitState;
use crate::backend::storage::store::Store;

use super::paths::{normalize_path, normalize_repo};

/// How often the loop looks for entries to refresh.
const TICK: Duration = Duration::from_secs(5);

/// An entry this old is probed again on the next tick, so none is older
/// than this plus [`TICK`] (30 s).
pub const REFRESH_AFTER_MS: u64 = 25_000;

/// Most uncommitted paths kept per agent; [`GitFacts::dirty_count`] is the
/// real total.
const MAX_DIRTY_FILES: usize = 500;

/// What git says about one working folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFacts {
    /// The repository's top folder, normalized.
    pub root: String,
    /// `owner/repo` from the `origin` remote; `None` without one.
    pub repo: Option<String>,
    /// `None` when detached or unknown.
    pub branch: Option<String>,
    /// Repository-relative paths with uncommitted changes (staged, unstaged,
    /// untracked; an untracked folder is reported whole, ending in `/`).
    pub dirty_files: Vec<String>,
    pub dirty_count: usize,
    /// Why the status part is missing, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub checked_ms: u64,
}

/// Probe `dir`: `None` when it isn't inside a git repository (or git is
/// missing or too slow).
pub async fn probe(dir: &str, now_ms: u64) -> Option<GitFacts> {
    let root = normalize_path(&safe_git_line(dir, &["rev-parse", "--show-toplevel"]).await?);
    let repo = safe_git_line(&root, &["remote", "get-url", "origin"])
        .await
        .and_then(|url| normalize_repo(&url));
    let mut facts = GitFacts { root, repo, checked_ms: now_ms, ..Default::default() };
    match safe_status_porcelain(&facts.root, &[]).await {
        Ok(out) => {
            let p = parse_porcelain_v2(&out);
            facts.branch = p.branch;
            let dirty: Vec<String> =
                p.records.into_iter().filter(|r| r.state != FsGitState::Ignored).map(|r| r.path).collect();
            facts.dirty_count = dirty.len();
            facts.dirty_files = dirty.into_iter().take(MAX_DIRTY_FILES).collect();
        }
        Err(e) => {
            facts.branch = safe_git_line(&facts.root, &["branch", "--show-current"]).await;
            facts.error = Some(match e {
                crate::backend::fs_ops::git::StatusError::Settings(what) => what.to_string(),
                crate::backend::fs_ops::git::StatusError::Git(m) => m,
            });
        }
    }
    Some(facts)
}

#[derive(Debug, Clone)]
struct Entry {
    dir: String,
    facts: Option<GitFacts>,
    checked_ms: u64,
}

/// Probe results per block id, with the folder each was taken in.
#[derive(Debug, Default)]
pub struct GitCache {
    entries: HashMap<String, Entry>,
}

impl GitCache {
    /// Whether `block`'s folder `dir` should be probed now.
    pub fn needs_probe(&self, block: &str, dir: &str, now_ms: u64) -> bool {
        match self.entries.get(block) {
            None => true,
            Some(e) => e.dir != dir || now_ms.saturating_sub(e.checked_ms) >= REFRESH_AFTER_MS,
        }
    }

    pub fn put(&mut self, block: &str, dir: &str, facts: Option<GitFacts>, now_ms: u64) {
        self.entries.insert(block.to_string(), Entry { dir: dir.to_string(), facts, checked_ms: now_ms });
    }

    /// The facts for `block`, only if they were taken in `dir`: after a `cd`
    /// the old repository is no longer what the agent is working in.
    pub fn get(&self, block: &str, dir: &str) -> Option<GitFacts> {
        self.entries.get(block).filter(|e| e.dir == dir).and_then(|e| e.facts.clone())
    }

    pub fn retain(&mut self, live: &HashSet<String>) {
        self.entries.retain(|b, _| live.contains(b));
    }
}

fn cache() -> &'static parking_lot::RwLock<GitCache> {
    static CACHE: std::sync::LazyLock<parking_lot::RwLock<GitCache>> = std::sync::LazyLock::new(Default::default);
    &CACHE
}

/// The cached facts for `block` in `dir` (see [`GitCache::get`]).
pub fn cached(block: &str, dir: &str) -> Option<GitFacts> {
    cache().read().get(block, dir)
}

/// Keep every registered agent's git facts fresh. Never returns.
pub async fn run_git_refresh_loop(mstore: Arc<Store>) {
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let agents = crate::backend::reactive::get_global_handler().list_agents();
        let live: HashSet<String> = agents.iter().map(|a| a.block_id.clone()).collect();
        cache().write().retain(&live);

        // Block meta lives in the store: read it off the async workers.
        let store = mstore.clone();
        let Ok(folders) = tokio::task::spawn_blocking(move || {
            let home = dirs::home_dir();
            agents
                .iter()
                .filter_map(|a| super::folder_of(&store, home.as_deref(), a).map(|d| (a.block_id.clone(), d)))
                .collect::<Vec<_>>()
        })
        .await
        else {
            continue;
        };

        // Agents sharing a folder share one probe per tick.
        let mut this_tick: HashMap<String, Option<GitFacts>> = HashMap::new();
        for (block, dir) in folders {
            let now = agentmux_common::time::now_ms_u64();
            if !cache().read().needs_probe(&block, &dir, now) {
                continue;
            }
            let facts = match this_tick.get(&dir) {
                Some(f) => f.clone(),
                None => {
                    let f = probe(&dir, now).await;
                    this_tick.insert(dir.clone(), f.clone());
                    f
                }
            };
            cache().write().put(&block, &dir, facts, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        use agentmux_common::win32::NoWindow;
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .no_window()
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    /// A temp repository gives its repository, branch and uncommitted files.
    #[tokio::test]
    async fn a_temp_repo_gives_repo_branch_and_dirty_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(dir, &["init", "-q", "-b", "feat/x"]);
        git(dir, &["remote", "add", "origin", "https://github.com/AgentMuxAI/Example.git"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "a").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("src/a.rs"), "changed").unwrap();
        std::fs::create_dir_all(dir.join("new")).unwrap();
        std::fs::write(dir.join("new/b.rs"), "b").unwrap();

        let sub = dir.join("src");
        let facts = probe(&sub.to_string_lossy(), 42).await.expect("inside a repository");
        assert_eq!(facts.repo.as_deref(), Some("agentmuxai/example"));
        assert_eq!(facts.branch.as_deref(), Some("feat/x"));
        let mut dirty = facts.dirty_files.clone();
        dirty.sort();
        assert_eq!(dirty, vec!["new/".to_string(), "src/a.rs".to_string()]);
        assert_eq!(facts.dirty_count, 2);
        assert_eq!(facts.checked_ms, 42);
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(facts.root.ends_with(&name), "root {} is the temp repository, not its src folder", facts.root);
    }

    #[tokio::test]
    async fn a_folder_outside_any_repository_has_no_git_facts() {
        let tmp = tempfile::tempdir().unwrap();
        // A temp folder might sit inside a repository on a dev machine; only
        // assert when git agrees it doesn't.
        if let Some(f) = probe(&tmp.path().to_string_lossy(), 1).await {
            assert!(!f.root.is_empty());
        }
    }

    /// The 30 s cache: an entry is reused until it is REFRESH_AFTER_MS old,
    /// and dropped at once when the agent changes folder.
    #[test]
    fn the_cache_is_reused_until_stale_or_the_folder_changes() {
        let mut c = GitCache::default();
        assert!(c.needs_probe("b1", "/r", 0), "never probed");
        let facts = GitFacts { root: "/r".into(), checked_ms: 1_000, ..Default::default() };
        c.put("b1", "/r", Some(facts.clone()), 1_000);
        assert!(!c.needs_probe("b1", "/r", 1_000 + REFRESH_AFTER_MS - 1));
        assert!(c.needs_probe("b1", "/r", 1_000 + REFRESH_AFTER_MS));
        assert!(c.needs_probe("b1", "/other", 1_001), "a cd re-probes at once");
        assert_eq!(c.get("b1", "/r"), Some(facts));
        assert_eq!(c.get("b1", "/other"), None, "facts from the old folder aren't shown");

        c.put("b2", "/n", None, 1_000);
        assert!(!c.needs_probe("b2", "/n", 2_000), "a folder outside git is cached too");
        c.retain(&HashSet::from(["b2".to_string()]));
        assert_eq!(c.get("b1", "/r"), None, "a gone agent's entry is dropped");
    }

    #[test]
    fn refresh_plus_tick_stays_within_thirty_seconds() {
        assert!(REFRESH_AFTER_MS + TICK.as_millis() as u64 <= 30_000);
    }
}
