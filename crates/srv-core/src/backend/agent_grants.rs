// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agents the person has told "stop asking" about one kind of notice
//! (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §4): this
//! agent may start agents, use a Google connector, adopt memory. "Always" on a
//! notice's card writes one here, through the operator-only
//! `agent.notice.decide`; Settings and the agent's pane menu revoke it.
//!
//! The grant belongs to the agent that was shown the card, by its definition
//! id (case-insensitive), in `agent-grants.json` under AgentMux's config dir.
//! Access to an SSH host is per agent and host, and stays in
//! `remote::agent_access`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "agent-grants.json";

/// What an agent is let do without a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// Start workers and create agents.
    Spawn,
    /// Use a Google connector whose definition asks to confirm.
    Google,
    /// Adopt memory from its earlier accounts.
    Memory,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Grants {
    #[serde(default)]
    always: Vec<Grant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Grant {
    topic: Topic,
    agent: String,
}

/// Serializes read-modify-write of the file within this srv.
static LOCK: Mutex<()> = Mutex::new(());

fn path_in(config_home: &Path) -> PathBuf {
    config_home.join(FILE_NAME)
}

fn read(path: &Path) -> Grants {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(config_home: &Path, grants: &Grants) -> std::io::Result<()> {
    std::fs::create_dir_all(config_home)?;
    let path = path_in(config_home);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(grants).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Whether the person said "always" to `topic` for `agent`.
pub fn allowed_in(config_home: &Path, topic: Topic, agent: &str) -> bool {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home)).always.iter().any(|g| g.topic == topic && g.agent.eq_ignore_ascii_case(agent))
}

/// Remember "always" to `topic` for `agent`.
pub fn allow_in(config_home: &Path, topic: Topic, agent: &str) -> std::io::Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut grants = read(&path_in(config_home));
    let grant = Grant { topic, agent: agent.to_ascii_lowercase() };
    if grants.always.contains(&grant) {
        return Ok(());
    }
    grants.always.push(grant);
    write(config_home, &grants)
}

/// Forget it: the agent gets the card again. `false` if there was no such grant.
pub fn revoke_in(config_home: &Path, topic: Topic, agent: &str) -> std::io::Result<bool> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut grants = read(&path_in(config_home));
    let before = grants.always.len();
    grants.always.retain(|g| !(g.topic == topic && g.agent.eq_ignore_ascii_case(agent)));
    if grants.always.len() == before {
        return Ok(false);
    }
    write(config_home, &grants)?;
    Ok(true)
}

/// Every agent let do `topic`.
pub fn all_in(config_home: &Path, topic: Topic) -> Vec<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home)).always.into_iter().filter(|g| g.topic == topic).map(|g| g.agent).collect()
}

/// This instance's config dir; in tests one temporary dir for the whole run, so no test writes the real one.
fn home() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        DIR.get_or_init(|| tempfile::tempdir().expect("a temporary config dir")).path().to_path_buf()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    crate::backend::base::get_mux_config_dir()
}

/// [`allowed_in`] under this instance's config dir.
pub fn allowed(topic: Topic, agent: &str) -> bool {
    allowed_in(&home(), topic, agent)
}

/// [`allow_in`] under this instance's config dir.
pub fn allow(topic: Topic, agent: &str) -> std::io::Result<()> {
    allow_in(&home(), topic, agent)
}

/// [`revoke_in`] under this instance's config dir.
pub fn revoke(topic: Topic, agent: &str) -> std::io::Result<bool> {
    revoke_in(&home(), topic, agent)
}

/// [`all_in`] under this instance's config dir.
pub fn all(topic: Topic) -> Vec<String> {
    all_in(&home(), topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_is_per_agent_and_topic() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(!allowed_in(home, Topic::Spawn, "korp"), "nothing until the person says so");
        allow_in(home, Topic::Spawn, "Korp").unwrap();
        allow_in(home, Topic::Spawn, "korp").unwrap();
        assert!(allowed_in(home, Topic::Spawn, "KORP"), "agent ids ignore case");
        assert!(!allowed_in(home, Topic::Google, "korp"), "another topic asks");
        assert!(!allowed_in(home, Topic::Spawn, "agentx"), "another agent asks");
        assert_eq!(std::fs::read_to_string(home.join(FILE_NAME)).unwrap().matches("korp").count(), 1, "stored once");
        assert_eq!(all_in(home, Topic::Spawn), vec!["korp".to_string()]);
    }

    #[test]
    fn revoking_asks_again_and_leaves_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        allow_in(home, Topic::Spawn, "korp").unwrap();
        allow_in(home, Topic::Google, "korp").unwrap();
        allow_in(home, Topic::Spawn, "agentx").unwrap();
        assert!(revoke_in(home, Topic::Spawn, "KORP").unwrap());
        assert!(!allowed_in(home, Topic::Spawn, "korp"));
        assert!(allowed_in(home, Topic::Google, "korp"), "another topic keeps its grant");
        assert!(allowed_in(home, Topic::Spawn, "agentx"), "another agent keeps its grant");
        assert!(!revoke_in(home, Topic::Spawn, "korp").unwrap(), "nothing left to revoke");
    }

    #[test]
    fn an_unreadable_file_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "not json").unwrap();
        assert!(!allowed_in(dir.path(), Topic::Spawn, "korp"));
    }
}
