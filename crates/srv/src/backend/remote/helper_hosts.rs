// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The SSH hosts AgentMux's helper has answered on (spec §7.1 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md): on such a host
//! a new terminal pane is durable unless the pane, the connection's settings
//! or the global setting say otherwise. Kept in `remote-helper-hosts.json`
//! under AgentMux's config dir; a host is its canonical connection name
//! (`ConnTarget::name`), so one host is one entry however it is spelled.
//!
//! The same file keeps what AgentMux learned about each host for the Remotes
//! pane (SPEC_REMOTES_PANE_2026_10_05.md §4.6): its platform (`uname -sm`, from
//! the helper install's probe) and the AgentMux version whose helper last
//! answered there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "remote-helper-hosts.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Hosts {
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    info: BTreeMap<String, HostInfo>,
}

/// What AgentMux learned about one host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    /// `uname -sm` as the host printed it (`Linux x86_64`, `Darwin arm64`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uname: String,
    /// The AgentMux version whose helper last answered here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub helper_version: String,
    /// When this record last changed, in Unix milliseconds.
    #[serde(default)]
    pub seen_at_ms: u64,
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

fn write(config_home: &Path, hosts: &Hosts) -> std::io::Result<()> {
    let path = path_in(config_home);
    std::fs::create_dir_all(config_home)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(hosts).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

fn canonical(connection: &str) -> Option<String> {
    match super::ConnTarget::parse(connection) {
        Ok(t @ super::ConnTarget::Ssh(_)) => Some(t.name()),
        _ => None,
    }
}

use agentmux_common::time::now_ms_u64 as now_ms;

/// Change `connection`'s record (and, with `helper`, mark the helper as there),
/// writing only when something changed.
fn change_in(
    config_home: &Path,
    connection: &str,
    helper: bool,
    edit: impl FnOnce(&mut HostInfo),
) -> std::io::Result<()> {
    let Some(name) = canonical(connection) else {
        return Ok(());
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut hosts = read(&path_in(config_home));
    let before = hosts.clone();
    if helper && !hosts.hosts.contains(&name) {
        hosts.hosts.push(name.clone());
    }
    let info = hosts.info.entry(name).or_default();
    let mut edited = info.clone();
    edit(&mut edited);
    if edited != *info {
        edited.seen_at_ms = now_ms();
        *info = edited;
    }
    if hosts == before {
        return Ok(());
    }
    write(config_home, &hosts)
}

/// Whether the helper has answered on `connection`'s host.
pub fn known_in(config_home: &Path, connection: &str) -> bool {
    let Some(name) = canonical(connection) else {
        return false;
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path_in(config_home)).hosts.contains(&name)
}

/// Remember that the helper of AgentMux `version` answered on `connection`'s host.
pub fn remember_in(config_home: &Path, connection: &str, version: &str) -> std::io::Result<()> {
    change_in(config_home, connection, true, |info| info.helper_version = version.to_string())
}

/// Record `connection`'s platform, as `uname -sm` printed it.
pub fn record_uname_in(config_home: &Path, connection: &str, uname: &str) -> std::io::Result<()> {
    let uname = uname.trim();
    if uname.is_empty() {
        return Ok(());
    }
    change_in(config_home, connection, false, |info| info.uname = uname.to_string())
}

/// Every host the helper has answered on, and every host's record, by canonical name.
pub fn all_in(config_home: &Path) -> (Vec<String>, BTreeMap<String, HostInfo>) {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let hosts = read(&path_in(config_home));
    (hosts.hosts, hosts.info)
}

/// [`known_in`] under this instance's config dir.
pub fn known(connection: &str) -> bool {
    known_in(&crate::backend::base::get_mux_config_dir(), connection)
}

/// [`remember_in`] under this instance's config dir, for this AgentMux's
/// version. Best effort: a host not remembered only means its next pane is
/// not durable by default.
pub fn remember(connection: &str) {
    if let Err(e) = remember_in(
        &crate::backend::base::get_mux_config_dir(),
        connection,
        env!("CARGO_PKG_VERSION"),
    ) {
        tracing::warn!(connection = %connection, error = %e, "could not remember a helper host");
    }
}

/// [`record_uname_in`] under this instance's config dir. Best effort.
pub fn record_uname(connection: &str, uname: &str) {
    if let Err(e) = record_uname_in(&crate::backend::base::get_mux_config_dir(), connection, uname) {
        tracing::warn!(connection = %connection, error = %e, "could not record a host's platform");
    }
}

/// The helper was removed from `connection`'s host: it is no longer a helper
/// host, and no version is there. Its platform stays known.
pub fn forget_helper_in(config_home: &Path, connection: &str) -> std::io::Result<()> {
    let Some(name) = canonical(connection) else {
        return Ok(());
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut hosts = read(&path_in(config_home));
    let before = hosts.clone();
    hosts.hosts.retain(|h| h != &name);
    if let Some(info) = hosts.info.get_mut(&name) {
        if !info.helper_version.is_empty() {
            info.helper_version.clear();
            info.seen_at_ms = now_ms();
        }
    }
    if hosts == before {
        return Ok(());
    }
    write(config_home, &hosts)
}

/// [`forget_helper_in`] under this instance's config dir.
pub fn forget_helper(connection: &str) -> std::io::Result<()> {
    forget_helper_in(&crate::backend::base::get_mux_config_dir(), connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_helper_is_forgotten_but_the_platform_kept() {
        let dir = tempfile::tempdir().unwrap();
        record_uname_in(dir.path(), "user@box", "Linux x86_64").unwrap();
        remember_in(dir.path(), "user@box", "0.59.9").unwrap();
        remember_in(dir.path(), "other@box", "0.59.9").unwrap();
        forget_helper_in(dir.path(), " user@box ").unwrap();
        assert!(!known_in(dir.path(), "user@box"));
        assert!(known_in(dir.path(), "other@box"), "other hosts untouched");
        let (_, info) = all_in(dir.path());
        assert_eq!(info["user@box"].uname, "Linux x86_64");
        assert_eq!(info["user@box"].helper_version, "");
    }

    #[test]
    fn a_host_is_remembered_once_however_it_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!known_in(dir.path(), "user@box"));
        remember_in(dir.path(), "user@box", "0.59.9").unwrap();
        remember_in(dir.path(), " user@box ", "0.59.9").unwrap();
        assert!(known_in(dir.path(), "user@box"));
        assert_eq!(read(&path_in(dir.path())).hosts.len(), 1);
        assert!(!known_in(dir.path(), "other@box"));
        // Not SSH: never a helper host.
        remember_in(dir.path(), "wsl://Ubuntu", "0.59.9").unwrap();
        assert!(!known_in(dir.path(), "wsl://Ubuntu"));
        assert!(!known_in(dir.path(), "local"));
    }

    #[test]
    fn a_hosts_platform_and_helper_version_are_kept_with_it() {
        let dir = tempfile::tempdir().unwrap();
        record_uname_in(dir.path(), "user@box", "Linux x86_64\n").unwrap();
        assert!(!known_in(dir.path(), "user@box"), "a platform alone is not the helper");
        remember_in(dir.path(), "user@box", "0.59.9").unwrap();
        let (hosts, info) = all_in(dir.path());
        assert_eq!(hosts, vec!["user@box".to_string()]);
        let box_info = &info["user@box"];
        assert_eq!(box_info.uname, "Linux x86_64");
        assert_eq!(box_info.helper_version, "0.59.9");
        assert!(box_info.seen_at_ms > 0);

        // A newer helper replaces the version; a blank uname changes nothing.
        remember_in(dir.path(), "user@box", "0.60.0").unwrap();
        record_uname_in(dir.path(), "user@box", "  ").unwrap();
        let (_, info) = all_in(dir.path());
        assert_eq!(info["user@box"].helper_version, "0.60.0");
        assert_eq!(info["user@box"].uname, "Linux x86_64");
    }

    #[test]
    fn a_file_from_before_the_records_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), r#"{ "hosts": ["user@box"] }"#).unwrap();
        assert!(known_in(dir.path(), "user@box"));
        let (_, info) = all_in(dir.path());
        assert!(info.is_empty());
    }
}
