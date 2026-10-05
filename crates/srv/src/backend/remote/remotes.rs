// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Remotes pane's list (SPEC_REMOTES_PANE_2026_10_05.md §4.6): every remote
//! machine AgentMux knows about, one record each, built from what srv already
//! keeps:
//! - the hosts named in `~/.ssh/config` (`ssh_config`);
//! - the per-connection settings (`settings.json` → `connections`);
//! - recent connections (`remote-recent.json`, kept here, §4.5);
//! - WSL distributions (`wsl`);
//! - each connection's status (`status`);
//! - where the helper is, and each host's platform (`helper_hosts`);
//! - which agents may use a host (`agent_access`);
//! - the last count of durable sessions seen on a host (kept here).
//!
//! Building the list never opens an ssh connection.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::helper_hosts::HostInfo;
use super::ConnTarget;
use crate::backend::mps::{Broker, MuxEvent, EVENT_REMOTES_CHANGE};
use crate::backend::rpc_types::{ConnStatus, RemoteHelper, RemotePlatform, RemoteRecord, RemoteStatus};
use crate::backend::wconfig::ConnKeywords;

// ── Recent connections (§4.5) ───────────────────────────────────────────────

const RECENT_FILE: &str = "remote-recent.json";
/// Recent connections kept; the least recently used goes first.
pub const MAX_RECENT: usize = 20;
/// A connection already recent is re-written at most this often.
const TOUCH_EVERY_MS: u64 = 60_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    recent: Vec<RecentEntry>,
}

/// One SSH connection that has reached `connected`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    /// Its canonical name (`ConnTarget::name`).
    pub name: String,
    pub first_used_ms: u64,
    pub last_used_ms: u64,
}

static RECENT_LOCK: Mutex<()> = Mutex::new(());

fn recent_path(config_home: &Path) -> PathBuf {
    config_home.join(RECENT_FILE)
}

fn read_recent(path: &Path) -> RecentFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_recent(config_home: &Path, file: &RecentFile) -> std::io::Result<()> {
    let path = recent_path(config_home);
    std::fs::create_dir_all(config_home)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(file).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

fn ssh_name(connection: &str) -> Option<String> {
    match ConnTarget::parse(connection) {
        Ok(t @ ConnTarget::Ssh(_)) => Some(t.name()),
        _ => None,
    }
}

/// Record that `connection` (SSH only) was used at `now_ms`. Returns whether the
/// list changed (a new entry); touching an existing one is written at most
/// once a minute.
pub fn record_recent_in(config_home: &Path, connection: &str, now_ms: u64) -> std::io::Result<bool> {
    let Some(name) = ssh_name(connection) else {
        return Ok(false);
    };
    let _g = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = read_recent(&recent_path(config_home));
    if let Some(entry) = file.recent.iter_mut().find(|e| e.name == name) {
        if now_ms.saturating_sub(entry.last_used_ms) < TOUCH_EVERY_MS {
            return Ok(false);
        }
        entry.last_used_ms = now_ms;
        write_recent(config_home, &file)?;
        return Ok(false);
    }
    file.recent.push(RecentEntry {
        name,
        first_used_ms: now_ms,
        last_used_ms: now_ms,
    });
    if file.recent.len() > MAX_RECENT {
        file.recent.sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms));
        file.recent.truncate(MAX_RECENT);
    }
    write_recent(config_home, &file)?;
    Ok(true)
}

/// Forget `connection` from the recent list. Returns whether it was there.
pub fn forget_recent_in(config_home: &Path, connection: &str) -> std::io::Result<bool> {
    let Some(name) = ssh_name(connection) else {
        return Ok(false);
    };
    let _g = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = read_recent(&recent_path(config_home));
    let before = file.recent.len();
    file.recent.retain(|e| e.name != name);
    if file.recent.len() == before {
        return Ok(false);
    }
    write_recent(config_home, &file)?;
    Ok(true)
}

/// The recent list, most recently used first.
pub fn recent_in(config_home: &Path) -> Vec<RecentEntry> {
    let _g = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut recent = read_recent(&recent_path(config_home)).recent;
    recent.sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms));
    recent
}

use agentmux_common::time::now_ms_u64 as now_ms;

/// [`record_recent_in`] under this instance's config dir, now. Best effort.
pub fn record_recent(connection: &str) -> bool {
    record_recent_in(&crate::backend::base::get_mux_config_dir(), connection, now_ms()).unwrap_or_else(|e| {
        tracing::warn!(connection = %connection, error = %e, "could not record a recent connection");
        false
    })
}

/// [`forget_recent_in`] under this instance's config dir.
pub fn forget_recent(connection: &str) -> std::io::Result<bool> {
    forget_recent_in(&crate::backend::base::get_mux_config_dir(), connection)
}

// ── Durable session counts (§4.6) ───────────────────────────────────────────

fn session_counts() -> &'static Mutex<HashMap<String, u32>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember how many durable sessions `connection`'s host had when last
/// listed (the list never opens ssh to count them).
pub fn note_sessions(connection: &str, count: usize) {
    if let Some(name) = ssh_name(connection) {
        session_counts()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name, count as u32);
    }
}

// ── Settings writes (§4.3) ──────────────────────────────────────────────────

/// Apply `values` to `connection`'s entry in the raw `connections` object of
/// `settings.json`: a `null` removes a key, an entry left empty is removed.
/// Works on the raw JSON, so keys AgentMux doesn't know survive untouched.
pub fn apply_connection_values(
    connections: &mut serde_json::Map<String, serde_json::Value>,
    connection: &str,
    values: &serde_json::Map<String, serde_json::Value>,
) {
    let name = connection.trim().to_string();
    let mut entry = match connections.remove(&name) {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    for (key, value) in values {
        if value.is_null() {
            entry.remove(key);
        } else {
            entry.insert(key.clone(), value.clone());
        }
    }
    if !entry.is_empty() {
        connections.insert(name, serde_json::Value::Object(entry));
    }
}

/// Refuse an edit that would leave `connection`'s entry unreadable (a string
/// where `display:pinned` wants a boolean, say): the write would land on disk
/// while the live config silently kept the old values.
pub fn check_connection_entry(
    connections: &serde_json::Map<String, serde_json::Value>,
    connection: &str,
) -> Result<(), String> {
    let name = connection.trim();
    match connections.get(name) {
        None => Ok(()),
        Some(entry) => serde_json::from_value::<ConnKeywords>(entry.clone())
            .map(|_| ())
            .map_err(|e| format!("invalid setting for {name}: {e}")),
    }
}

/// Tell every Remotes pane its list changed.
pub fn publish_change(broker: &Broker) {
    broker.publish(MuxEvent {
        event: EVENT_REMOTES_CHANGE.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
}

// ── The list (§4.6) ─────────────────────────────────────────────────────────

/// Everything [`build`] reads, gathered by [`list`].
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub ssh_config_hosts: Vec<String>,
    pub wsl_distros: Vec<String>,
    pub settings: HashMap<String, ConnKeywords>,
    pub recent: Vec<RecentEntry>,
    pub statuses: Vec<ConnStatus>,
    pub helper_hosts: Vec<String>,
    pub host_info: BTreeMap<String, HostInfo>,
    pub sessions: HashMap<String, u32>,
    pub grants: Vec<(String, String)>,
}

/// `uname -sm` → platform.
pub fn platform_of(uname: &str) -> Option<RemotePlatform> {
    let mut words = uname.split_whitespace();
    let os = words.next()?;
    let arch = words.next().unwrap_or_default();
    let os = match os {
        "Linux" => "linux".to_string(),
        "Darwin" => "macos".to_string(),
        other => other.to_ascii_lowercase(),
    };
    let arch = match arch {
        "x86_64" | "amd64" => "x86_64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        other => other.to_string(),
    };
    Some(RemotePlatform { os, arch })
}

/// The canonical name and kind of a connection name, or `None` for `local`.
fn identify(connection: &str) -> Option<(String, &'static str)> {
    match ConnTarget::parse(connection).ok()? {
        ConnTarget::Local => None,
        t @ ConnTarget::Wsl(_) => Some((t.name(), "wsl")),
        t @ ConnTarget::Ssh(_) => Some((t.name(), "ssh")),
    }
}

/// One record per remote, by name (the pane sorts them into sections).
pub fn build(inputs: &Inputs) -> Vec<RemoteRecord> {
    // name → (kind, sources)
    let mut found: BTreeMap<String, (&'static str, Vec<&'static str>)> = BTreeMap::new();
    let mut add = |connection: &str, source: &'static str| {
        if let Some((name, kind)) = identify(connection) {
            let entry = found.entry(name).or_insert((kind, Vec::new()));
            if !entry.1.contains(&source) {
                entry.1.push(source);
            }
        }
    };
    for host in &inputs.ssh_config_hosts {
        add(host, "ssh_config");
    }
    for name in inputs.settings.keys() {
        add(name, "settings");
    }
    for entry in &inputs.recent {
        add(&entry.name, "recent");
    }
    for distro in &inputs.wsl_distros {
        add(&format!("wsl://{distro}"), "wsl");
    }

    let same = |a: &str, b: &str| super::conn::same_connection(a, b);
    found
        .into_iter()
        .map(|(name, (kind, sources))| {
            let status = inputs
                .statuses
                .iter()
                .find(|s| same(&s.connection, &name))
                .map(|s| RemoteStatus {
                    state: s.status.clone(),
                    error: s.error.clone(),
                })
                .unwrap_or_else(|| RemoteStatus {
                    state: super::status::state::DISCONNECTED.to_string(),
                    error: String::new(),
                });
            let settings = inputs.settings.iter().find(|(k, _)| same(k, &name)).map(|(_, v)| v);
            let info = inputs.host_info.get(&name);
            let platform = info.and_then(|i| platform_of(&i.uname));
            let helper = if kind == "wsl" {
                RemoteHelper { state: "none".to_string(), version: String::new() }
            } else {
                let version = info.map(|i| i.helper_version.clone()).unwrap_or_default();
                let state = if settings.is_some_and(|s| s.conn_helper == "never") {
                    "never"
                } else if inputs.helper_hosts.contains(&name) {
                    "installed"
                } else if info.is_some_and(|i| {
                    !i.uname.is_empty() && super::helper_install::target_for(&i.uname).is_none()
                }) {
                    "unsupported"
                } else {
                    "absent"
                };
                let version = if state == "installed" { version } else { String::new() };
                RemoteHelper { state: state.to_string(), version }
            };
            let mut agents: Vec<String> = inputs
                .grants
                .iter()
                .filter(|(_, conn)| same(conn, &name))
                .map(|(agent, _)| agent.clone())
                .collect();
            agents.sort();
            agents.dedup();
            RemoteRecord {
                sessions: inputs.sessions.get(&name).copied(),
                last_used_ms: inputs.recent.iter().find(|e| e.name == name).map(|e| e.last_used_ms),
                settings: settings
                    .and_then(|s| serde_json::to_value(s).ok())
                    .unwrap_or_else(|| serde_json::json!({})),
                name,
                kind: kind.to_string(),
                sources: sources.into_iter().map(str::to_string).collect(),
                status,
                platform,
                helper,
                agents,
            }
        })
        .collect()
}

/// The list now, for `RemotesList`. `settings` is `FullConfigType::connections`.
pub async fn list(settings: HashMap<String, ConnKeywords>) -> Vec<RemoteRecord> {
    let ssh_config_hosts = tokio::task::spawn_blocking(super::ssh_config::hosts)
        .await
        .unwrap_or_default();
    let wsl_distros = super::wsl::list_cached().await;
    let config_home = crate::backend::base::get_mux_config_dir();
    let (helper_hosts, host_info) = super::helper_hosts::all_in(&config_home);
    let inputs = Inputs {
        ssh_config_hosts,
        wsl_distros,
        settings,
        recent: recent_in(&config_home),
        statuses: super::status::all(),
        helper_hosts,
        host_info,
        sessions: session_counts().lock().unwrap_or_else(|e| e.into_inner()).clone(),
        grants: super::agent_access::all_in(&config_home),
    };
    build(&inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(connection: &str, state: &str) -> ConnStatus {
        ConnStatus {
            status: state.to_string(),
            connection: connection.to_string(),
            connected: state == "connected",
            hasconnected: true,
            activeconnnum: 0,
            error: String::new(),
        }
    }

    fn named<'a>(records: &'a [RemoteRecord], name: &str) -> &'a RemoteRecord {
        records.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no record for {name}"))
    }

    #[test]
    fn one_record_per_remote_whatever_lists_it() {
        let mut settings = HashMap::new();
        settings.insert("db1".to_string(), ConnKeywords::default());
        settings.insert("local".to_string(), ConnKeywords::default());
        let inputs = Inputs {
            ssh_config_hosts: vec!["db1".into(), "web".into()],
            wsl_distros: vec!["Ubuntu".into()],
            settings,
            recent: vec![
                RecentEntry { name: "db1".into(), first_used_ms: 1, last_used_ms: 5 },
                RecentEntry { name: "me@10.0.0.7".into(), first_used_ms: 2, last_used_ms: 9 },
            ],
            ..Default::default()
        };
        let records = build(&inputs);
        let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["db1", "me@10.0.0.7", "web", "wsl://Ubuntu"], "local is never a remote");
        assert_eq!(named(&records, "db1").sources, ["ssh_config", "settings", "recent"]);
        assert_eq!(named(&records, "me@10.0.0.7").sources, ["recent"]);
        assert_eq!(named(&records, "me@10.0.0.7").last_used_ms, Some(9));
        let wsl = named(&records, "wsl://Ubuntu");
        assert_eq!((wsl.kind.as_str(), wsl.helper.state.as_str()), ("wsl", "none"));
        assert_eq!(named(&records, "web").status.state, "disconnected", "no status yet");
    }

    #[test]
    fn a_record_carries_status_platform_helper_sessions_and_agents() {
        let mut host_info = BTreeMap::new();
        host_info.insert(
            "db1".to_string(),
            HostInfo { uname: "Linux aarch64".into(), helper_version: "0.59.9".into(), seen_at_ms: 1 },
        );
        host_info.insert("router".to_string(), HostInfo { uname: "FreeBSD amd64".into(), ..Default::default() });
        let mut sessions = HashMap::new();
        sessions.insert("db1".to_string(), 2);
        let inputs = Inputs {
            ssh_config_hosts: vec!["db1".into(), "router".into(), "web".into()],
            statuses: vec![status("db1", "connected")],
            helper_hosts: vec!["db1".into()],
            host_info,
            sessions,
            grants: vec![("korp".into(), "db1".into()), ("camper".into(), " db1".into()), ("korp".into(), "web".into())],
            ..Default::default()
        };
        let records = build(&inputs);
        let db1 = named(&records, "db1");
        assert_eq!(db1.status.state, "connected");
        assert_eq!(db1.platform, Some(RemotePlatform { os: "linux".into(), arch: "arm64".into() }));
        assert_eq!((db1.helper.state.as_str(), db1.helper.version.as_str()), ("installed", "0.59.9"));
        assert_eq!(db1.sessions, Some(2));
        assert_eq!(db1.agents, ["camper", "korp"]);
        assert_eq!(named(&records, "router").helper.state, "unsupported", "no build for FreeBSD");
        let web = named(&records, "web");
        assert_eq!((web.helper.state.as_str(), web.sessions, web.platform.clone()), ("absent", None, None));
    }

    #[test]
    fn never_wins_over_an_installed_helper() {
        let mut settings = HashMap::new();
        settings.insert("db1".to_string(), ConnKeywords { conn_helper: "never".into(), ..Default::default() });
        let inputs = Inputs {
            ssh_config_hosts: vec!["db1".into()],
            settings,
            helper_hosts: vec!["db1".into()],
            ..Default::default()
        };
        let db1 = &build(&inputs)[0];
        assert_eq!(db1.helper.state, "never");
        assert!(db1.helper.version.is_empty());
    }

    #[test]
    fn a_records_settings_are_its_stored_keys_only() {
        let mut settings = HashMap::new();
        settings.insert(
            "db1".to_string(),
            ConnKeywords {
                display_name: "prod-db".into(),
                display_color: "#e5484d".into(),
                display_pinned: Some(true),
                ..Default::default()
            },
        );
        let inputs = Inputs { settings, ..Default::default() };
        let db1 = &build(&inputs)[0];
        assert_eq!(
            db1.settings,
            serde_json::json!({ "display:name": "prod-db", "display:color": "#e5484d", "display:pinned": true })
        );
    }

    #[test]
    fn platforms_read_from_uname() {
        assert_eq!(platform_of("Darwin arm64"), Some(RemotePlatform { os: "macos".into(), arch: "arm64".into() }));
        assert_eq!(platform_of("Linux x86_64\n"), Some(RemotePlatform { os: "linux".into(), arch: "x86_64".into() }));
        assert_eq!(platform_of(""), None);
    }

    #[test]
    fn recent_keeps_ssh_connections_touched_at_most_once_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(record_recent_in(home, "me@box", 1_000).unwrap(), "new");
        assert!(!record_recent_in(home, " me@box ", 2_000).unwrap(), "same host, same minute");
        assert_eq!(recent_in(home)[0].last_used_ms, 1_000, "not re-written within a minute");
        assert!(!record_recent_in(home, "me@box", 70_000).unwrap());
        assert_eq!(recent_in(home)[0].last_used_ms, 70_000);
        assert!(!record_recent_in(home, "wsl://Ubuntu", 1).unwrap(), "not SSH");
        assert!(!record_recent_in(home, "local", 1).unwrap());
        assert_eq!(recent_in(home).len(), 1);

        assert!(forget_recent_in(home, "me@box").unwrap());
        assert!(!forget_recent_in(home, "me@box").unwrap());
        assert!(recent_in(home).is_empty());
    }

    #[test]
    fn recent_drops_the_least_recently_used_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_RECENT as u64 + 3) {
            record_recent_in(dir.path(), &format!("me@host{i}"), 1_000 * (i + 1)).unwrap();
        }
        let recent = recent_in(dir.path());
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0].name, format!("me@host{}", MAX_RECENT + 2), "newest first");
        assert!(!recent.iter().any(|e| e.name == "me@host0"), "oldest dropped");
    }

    #[test]
    fn a_settings_write_changes_only_the_keys_given() {
        let mut connections = serde_json::json!({
            "db1": { "display:name": "old", "ssh:user": "admin", "custom:key": 7 },
            "web": { "term:theme": "red" }
        })
        .as_object()
        .unwrap()
        .clone();
        let values = serde_json::json!({ "display:name": "prod-db", "display:color": "#e5484d", "term:theme": null })
            .as_object()
            .unwrap()
            .clone();
        apply_connection_values(&mut connections, " db1 ", &values);
        assert_eq!(
            connections["db1"],
            serde_json::json!({ "display:name": "prod-db", "display:color": "#e5484d", "ssh:user": "admin", "custom:key": 7 }),
            "keys AgentMux doesn't know are kept"
        );
        assert_eq!(connections["web"], serde_json::json!({ "term:theme": "red" }), "other connections untouched");

        // Removing the last key removes the entry.
        let clear = serde_json::json!({ "term:theme": null }).as_object().unwrap().clone();
        apply_connection_values(&mut connections, "web", &clear);
        assert!(!connections.contains_key("web"));

        // A new connection.
        let pin = serde_json::json!({ "display:pinned": true }).as_object().unwrap().clone();
        apply_connection_values(&mut connections, "me@box", &pin);
        assert_eq!(connections["me@box"], serde_json::json!({ "display:pinned": true }));
    }

    #[test]
    fn an_edit_that_would_not_parse_is_refused() {
        let connections = serde_json::json!({
            "ok": { "display:pinned": true, "custom:key": 7 },
            "bad": { "display:pinned": "yes" }
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(check_connection_entry(&connections, "ok").is_ok(), "unknown keys are fine");
        assert!(check_connection_entry(&connections, "gone").is_ok(), "a removed entry is fine");
        let err = check_connection_entry(&connections, " bad ").unwrap_err();
        assert!(err.contains("bad"), "{err}");
    }
}
