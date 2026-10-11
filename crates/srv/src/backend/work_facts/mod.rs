// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Work facts: per running agent, what srv can already see or cheaply derive
//! about what it is working on, so agents can find out who else is working
//! on the same file, branch or topic
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.1).
//!
//! | Fact | Source |
//! |---|---|
//! | goal | block meta `term:ambient_summary`, else `term:last_prompt` |
//! | todos, current tool, recent files | the progress watcher (`reactive::progress_watcher`) |
//! | working folder | bashwrap's cwd state file, else the start folder (`cmd:cwd`) |
//! | repo, branch, uncommitted files | git in that folder, cached ([`git`]) |
//!
//! Files are compared across clones as (repository, repository-relative
//! path), since every agent has its own clone. Nothing here blocks anyone:
//! the facts only inform.

pub mod git;
pub mod matching;
pub mod paths;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::backend::blockcontroller::{cmd_env_of, get_block_controller_status, META_KEY_CMD_CWD, STATUS_RUNNING};
use crate::backend::obj::{self, Block, MetaMapType};
use crate::backend::reactive::progress_watcher::{watched_work, TodoItem};
use crate::backend::reactive::types::AgentRegistration;
use crate::backend::storage::store::Store;

/// The block meta key the agent pane writes the latest prompt to
/// (`frontend/app/store/swarm-line.ts`, `META_LAST_PROMPT`).
const META_LAST_PROMPT: &str = "term:last_prompt";

/// One file an agent changed recently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactFile {
    /// Normalized absolute path, as the tool call named it.
    pub path: String,
    /// Relative to the agent's repository, when the file is inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    pub tool: String,
    pub ts_ms: u64,
}

/// Everything known about one agent's current work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkFacts {
    pub agent: String,
    pub block_id: String,
    /// The AgentMux channel the agent runs in (`stable`, `dev`, ...).
    pub channel: String,
    /// `busy` (a turn is running), `idle`, or `stopped`.
    pub status: String,
    pub goal: Option<String>,
    pub working_folder: Option<String>,
    pub repo: Option<String>,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    /// Repository-relative, at most a few hundred; `dirty_count` is the total.
    pub dirty_files: Vec<String>,
    pub dirty_count: usize,
    /// Newest first.
    pub recent_files: Vec<FactFile>,
    pub todos: Vec<TodoItem>,
    pub current_tool: Option<String>,
    /// When git was last asked; `None` before the first probe or outside a
    /// repository.
    pub git_checked_ms: Option<u64>,
}

impl WorkFacts {
    /// The key two agents share when they work in the same repository: the
    /// `owner/repo` of its origin, else (no remote) the clone's own folder.
    pub fn repo_key(&self) -> Option<String> {
        self.repo.clone().or_else(|| self.repo_root.as_ref().map(|r| format!("local:{r}")))
    }
}

/// The agent's goal: its generated title, else its last prompt.
pub fn goal_of(meta: &MetaMapType) -> Option<String> {
    let title = obj::meta_get_string(meta, crate::ambient::title::META_TITLE, "");
    if crate::ambient::validate::is_usable_title(&title) {
        return Some(title.trim().to_string());
    }
    let prompt = obj::meta_get_string(meta, META_LAST_PROMPT, "");
    let prompt = prompt.trim();
    (!prompt.is_empty()).then(|| prompt.to_string())
}

/// The agent's working folder (see [`paths::working_folder`]), from its
/// block row. Reads the store: call off the async workers.
pub fn folder_of(store: &Store, home: Option<&Path>, reg: &AgentRegistration) -> Option<String> {
    let block = store.get::<Block>(&reg.block_id).ok()??;
    folder_of_meta(&block.meta, home, &reg.agent_id)
}

fn folder_of_meta(meta: &MetaMapType, home: Option<&Path>, agent_id: &str) -> Option<String> {
    let env = cmd_env_of(meta);
    let start = obj::meta_get_string(meta, META_KEY_CMD_CWD, "");
    paths::working_folder(home, &env, agent_id, &start)
}

fn status_of(block_id: &str) -> &'static str {
    match get_block_controller_status(block_id) {
        Some(s) if s.shellprocstatus == STATUS_RUNNING => {
            if s.turn_active {
                "busy"
            } else {
                "idle"
            }
        }
        _ => "stopped",
    }
}

/// The facts for every agent registered on this srv. Reads the store and
/// the caches only, never git: call off the async workers.
pub fn collect_local(store: &Store, regs: &[AgentRegistration], channel: &str) -> Vec<WorkFacts> {
    let home = dirs::home_dir();
    regs.iter()
        .map(|reg| {
            let meta = store.get::<Block>(&reg.block_id).ok().flatten().map(|b| b.meta).unwrap_or_default();
            let folder = folder_of_meta(&meta, home.as_deref(), &reg.agent_id);
            let git = folder.as_deref().and_then(|d| git::cached(&reg.block_id, d));
            let watched = watched_work(&reg.block_id).unwrap_or_default();
            build(
                reg,
                channel,
                status_of(&reg.block_id),
                goal_of(&meta),
                folder,
                git,
                watched,
            )
        })
        .collect()
}

/// Assemble one agent's facts from what each source said.
pub fn build(
    reg: &AgentRegistration,
    channel: &str,
    status: &str,
    goal: Option<String>,
    working_folder: Option<String>,
    git: Option<git::GitFacts>,
    watched: crate::backend::reactive::progress_watcher::WatchedWork,
) -> WorkFacts {
    let root = git.as_ref().map(|g| g.root.clone());
    let recent_files = watched
        .recent_files
        .into_iter()
        .map(|f| {
            let path = paths::normalize_path(&f.path);
            let rel = root.as_deref().and_then(|r| paths::relative_to(&path, r)).filter(|r| !r.is_empty());
            FactFile { path, rel, tool: f.tool, ts_ms: f.ts_ms }
        })
        .collect();
    let git = git.unwrap_or_default();
    WorkFacts {
        agent: reg.agent_id.clone(),
        block_id: reg.block_id.clone(),
        channel: channel.to_string(),
        status: status.to_string(),
        goal,
        working_folder,
        repo: git.repo,
        repo_root: root,
        branch: git.branch,
        dirty_files: git.dirty_files,
        dirty_count: git.dirty_count,
        recent_files,
        todos: watched.progress.todos,
        current_tool: watched.progress.current_tool,
        git_checked_ms: (git.checked_ms > 0).then_some(git.checked_ms),
    }
}

/// How many uncommitted and recent files a `ListConversations` entry shows.
const SUMMARY_DIRTY: usize = 10;
const SUMMARY_RECENT: usize = 5;

/// The facts as a `ListConversations` entry carries them (its `work` field):
/// the first few files of each list and a checklist summary.
pub fn conversation_summary(f: &WorkFacts) -> serde_json::Value {
    let done = f.todos.iter().filter(|t| t.status == "completed").count();
    let in_progress: Vec<&str> =
        f.todos.iter().filter(|t| t.status == "in_progress").map(|t| t.text.as_str()).take(3).collect();
    serde_json::json!({
        "goal": f.goal,
        "working_folder": f.working_folder,
        "repo": f.repo,
        "branch": f.branch,
        "dirty_count": f.dirty_count,
        "dirty_files": f.dirty_files.iter().take(SUMMARY_DIRTY).collect::<Vec<_>>(),
        "recent_files": f.recent_files.iter().take(SUMMARY_RECENT).map(|r| serde_json::json!({
            "path": r.rel.as_deref().unwrap_or(&r.path),
            "tool": r.tool,
            "ts_ms": r.ts_ms,
        })).collect::<Vec<_>>(),
        "todos": { "total": f.todos.len(), "completed": done, "in_progress": in_progress },
        "current_tool": f.current_tool,
        "git_checked_ms": f.git_checked_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::reactive::progress_watcher::{AgentProgress, RecentFile, WatchedWork};

    fn reg(agent: &str, block: &str) -> AgentRegistration {
        AgentRegistration {
            agent_id: agent.to_string(),
            block_id: block.to_string(),
            tab_id: None,
            uid: None,
            registered_at: 0,
            last_seen: 0,
            registration_nonce: 0,
        }
    }

    fn meta(pairs: &[(&str, &str)]) -> MetaMapType {
        pairs.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect()
    }

    #[test]
    fn the_goal_is_the_title_else_the_last_prompt() {
        assert_eq!(
            goal_of(&meta(&[("term:ambient_summary", "Fix the login race"), ("term:last_prompt", "hi")])).as_deref(),
            Some("Fix the login race")
        );
        assert_eq!(
            goal_of(&meta(&[("term:ambient_summary", ""), ("term:last_prompt", " add the allowlist ")])).as_deref(),
            Some("add the allowlist")
        );
        assert_eq!(goal_of(&meta(&[])), None);
    }

    #[test]
    fn build_maps_recent_files_into_the_repository_and_keeps_watcher_state() {
        let git = git::GitFacts {
            root: "C:/w/clone".into(),
            repo: Some("o/r".into()),
            branch: Some("feat".into()),
            dirty_files: vec!["a.rs".into()],
            dirty_count: 1,
            error: None,
            checked_ms: 7,
        };
        let watched = WatchedWork {
            progress: AgentProgress {
                todos: vec![TodoItem { text: "Ship".into(), status: "in_progress".into() }],
                current_tool: Some("Edit".into()),
                ..Default::default()
            },
            recent_files: vec![
                RecentFile { path: r"C:\w\clone\src\b.rs".into(), tool: "Edit".into(), ts_ms: 5 },
                RecentFile { path: "D:/elsewhere/c.rs".into(), tool: "Write".into(), ts_ms: 4 },
            ],
        };
        let f = build(&reg("A", "b1"), "stable", "idle", None, Some("C:/w/clone".into()), Some(git), watched);
        assert_eq!(f.repo.as_deref(), Some("o/r"));
        assert_eq!(f.recent_files[0].rel.as_deref(), Some("src/b.rs"));
        assert_eq!(f.recent_files[0].path, "C:/w/clone/src/b.rs");
        assert_eq!(f.recent_files[1].rel, None, "outside the repository");
        assert_eq!(f.current_tool.as_deref(), Some("Edit"));
        assert_eq!(f.git_checked_ms, Some(7));
        assert_eq!(f.repo_key().as_deref(), Some("o/r"));

        let summary = conversation_summary(&f);
        assert_eq!(summary["dirty_count"], 1);
        assert_eq!(summary["recent_files"][0]["path"], "src/b.rs");
        assert_eq!(summary["todos"]["in_progress"][0], "Ship");
    }

    #[test]
    fn a_repository_without_a_remote_is_keyed_by_its_folder() {
        let f = WorkFacts { repo_root: Some("C:/w/x".into()), ..Default::default() };
        assert_eq!(f.repo_key().as_deref(), Some("local:C:/w/x"));
        assert_eq!(WorkFacts::default().repo_key(), None);
    }

    #[test]
    fn facts_round_trip_for_the_cross_channel_endpoint() {
        let f = WorkFacts {
            agent: "A".into(),
            todos: vec![TodoItem { text: "t".into(), status: "pending".into() }],
            recent_files: vec![FactFile { path: "/x".into(), rel: None, tool: "Edit".into(), ts_ms: 1 }],
            ..Default::default()
        };
        let back: WorkFacts = serde_json::from_value(serde_json::to_value(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }
}
