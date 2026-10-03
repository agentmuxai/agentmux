// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The SSH hosts AgentMux's helper has answered on (spec §7.1 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md): on such a host
//! a new terminal pane is durable unless the pane, the connection's settings
//! or the global setting say otherwise. Kept in `remote-helper-hosts.json`
//! under AgentMux's config dir; a host is its canonical connection name
//! (`ConnTarget::name`), so one host is one entry however it is spelled.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "remote-helper-hosts.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Hosts {
    #[serde(default)]
    hosts: Vec<String>,
}

/// Serializes read-modify-write of the file within this srv.
static LOCK: Mutex<()> = Mutex::new(());

fn path_in(config_home: &Path) -> PathBuf {
    config_home.join(FILE_NAME)
}

fn read(path: &Path) -> Hosts {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn canonical(connection: &str) -> Option<String> {
    match super::ConnTarget::parse(connection) {
        Ok(t @ super::ConnTarget::Ssh(_)) => Some(t.name()),
        _ => None,
    }
}

/// Whether the helper has answered on `connection`'s host.
pub fn known_in(config_home: &Path, connection: &str) -> bool {
    let Some(name) = canonical(connection) else {
        return false;
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home)).hosts.contains(&name)
}

/// Remember that the helper answered on `connection`'s host.
pub fn remember_in(config_home: &Path, connection: &str) -> std::io::Result<()> {
    let Some(name) = canonical(connection) else {
        return Ok(());
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = path_in(config_home);
    let mut hosts = read(&path);
    if hosts.hosts.contains(&name) {
        return Ok(());
    }
    hosts.hosts.push(name);
    std::fs::create_dir_all(config_home)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&hosts).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// [`known_in`] under this instance's config dir.
pub fn known(connection: &str) -> bool {
    known_in(&crate::backend::base::get_mux_config_dir(), connection)
}

/// [`remember_in`] under this instance's config dir. Best effort: a host
/// not remembered only means its next pane is not durable by default.
pub fn remember(connection: &str) {
    if let Err(e) = remember_in(&crate::backend::base::get_mux_config_dir(), connection) {
        tracing::warn!(connection = %connection, error = %e, "could not remember a helper host");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_remembered_once_however_it_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!known_in(dir.path(), "user@box"));
        remember_in(dir.path(), "user@box").unwrap();
        remember_in(dir.path(), " user@box ").unwrap();
        assert!(known_in(dir.path(), "user@box"));
        assert_eq!(read(&path_in(dir.path())).hosts.len(), 1);
        assert!(!known_in(dir.path(), "other@box"));
        // Not SSH: never a helper host.
        remember_in(dir.path(), "wsl://Ubuntu").unwrap();
        assert!(!known_in(dir.path(), "wsl://Ubuntu"));
        assert!(!known_in(dir.path(), "local"));
    }
}
