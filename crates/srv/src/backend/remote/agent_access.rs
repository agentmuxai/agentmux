// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which agents the user has let use which SSH hosts (spec §8.2 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! An agent on another machine uses the user's SSH identity, so access is
//! opt-in per agent and host: the first time, the user is asked (which agent,
//! which host, what it wants to run) and answers once or always. "Always" is
//! remembered here, in `ssh-agent-access.json` under AgentMux's config dir;
//! "once" is not stored at all. A host is its connection name, trimmed and
//! compared exactly (an ssh config alias is case-sensitive); an agent id
//! ignores case.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "ssh-agent-access.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Grants {
    #[serde(default)]
    always: Vec<Grant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Grant {
    agent: String,
    connection: String,
}

/// Serializes read-modify-write of the file within this srv.
static LOCK: Mutex<()> = Mutex::new(());

fn path_in(config_home: &Path) -> PathBuf {
    config_home.join(FILE_NAME)
}

fn read(path: &Path) -> Grants {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Whether the user said "always" for `agent` on `connection`.
pub fn always_allowed_in(config_home: &Path, agent: &str, connection: &str) -> bool {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home))
        .always
        .iter()
        .any(|g| g.agent.eq_ignore_ascii_case(agent) && g.connection == connection.trim())
}

/// Remember "always" for `agent` on `connection`.
pub fn remember_in(config_home: &Path, agent: &str, connection: &str) -> std::io::Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = path_in(config_home);
    let mut grants = read(&path);
    let grant = Grant {
        agent: agent.to_ascii_lowercase(),
        connection: connection.trim().to_string(),
    };
    if !grants.always.contains(&grant) {
        grants.always.push(grant);
    }
    std::fs::create_dir_all(config_home)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&grants).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// [`always_allowed_in`] under this instance's config dir.
pub fn always_allowed(agent: &str, connection: &str) -> bool {
    always_allowed_in(
        &crate::backend::base::get_mux_config_dir(),
        agent,
        connection,
    )
}

/// [`remember_in`] under this instance's config dir.
pub fn remember(agent: &str, connection: &str) -> std::io::Result<()> {
    remember_in(
        &crate::backend::base::get_mux_config_dir(),
        agent,
        connection,
    )
}

/// Every "always" grant, as `(agent, connection)` (the Remotes pane lists them
/// per host, SPEC_REMOTES_PANE_2026_10_05.md §4.10).
pub fn all_in(config_home: &Path) -> Vec<(String, String)> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home))
        .always
        .into_iter()
        .map(|g| (g.agent, g.connection))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_is_remembered_per_agent_and_host() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(
            !always_allowed_in(home, "korp", "area54"),
            "nothing until the user says so"
        );
        remember_in(home, "Korp", "area54").unwrap();
        remember_in(home, "korp", "area54").unwrap();
        assert!(always_allowed_in(home, "korp", "area54"));
        assert!(
            always_allowed_in(home, "KORP", " area54 "),
            "agent ids ignore case"
        );
        assert!(
            !always_allowed_in(home, "korp", "Area54"),
            "host names are exact"
        );
        assert!(
            !always_allowed_in(home, "agentx", "area54"),
            "another agent is asked"
        );
        assert!(
            !always_allowed_in(home, "korp", "nas"),
            "another host is asked"
        );
        let text = std::fs::read_to_string(home.join(FILE_NAME)).unwrap();
        assert_eq!(text.matches("area54").count(), 1, "stored once: {text}");
    }

    #[test]
    fn an_unreadable_file_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "not json").unwrap();
        assert!(!always_allowed_in(dir.path(), "korp", "area54"));
    }
}
