// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agents a LAN peer has listed, kept after the peer goes quiet.
//!
//! A peer that stops answering is dropped from `LanDiscoveryController::get_instances`, and its agents
//! with it, so a message to one of them looks like a message to a name nobody knows. This remembers the
//! name and the host that listed it, so `server::reactive::peer_hold` can tell "an agent of a machine that is
//! down" from "a name that does not exist". Kept in `lan-seen-agents.json` beside the databases, written
//! only when a name appears or moves host (or its record is a day old), so a poll every few seconds
//! costs no writes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// Names kept; the oldest listing is dropped past this, so a peer listing endless names cannot grow it.
const MAX_SEEN: usize = 1024;
/// A name not listed for this long is forgotten: the agent was deleted or its machine retired.
const FORGET_AFTER_SECS: u64 = 30 * 24 * 3600;
/// A listing that changes nothing is written again only when the stored one is this old.
const REFRESH_AFTER_SECS: u64 = 24 * 3600;

/// Where a name was last listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenAgent {
    pub hostname: String,
    pub seen_secs: u64,
}

#[derive(Default)]
struct Seen {
    agents: HashMap<String, SeenAgent>,
    loaded: bool,
}

impl Seen {
    /// Note that `hostname` listed `names` at `now_secs`; whether the record on disk is now out of date.
    fn remember(&mut self, names: &[String], hostname: &str, now_secs: u64) -> bool {
        let mut stale = false;
        for name in names {
            let key = name.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let old = self.agents.get(&key);
            if old.is_none_or(|o| o.hostname != hostname || now_secs.saturating_sub(o.seen_secs) >= REFRESH_AFTER_SECS) {
                stale = true;
                self.agents.insert(key, SeenAgent { hostname: hostname.to_string(), seen_secs: now_secs });
            }
        }
        while self.agents.len() > MAX_SEEN {
            let Some(oldest) = self.agents.iter().min_by_key(|(_, v)| v.seen_secs).map(|(k, _)| k.clone()) else { break };
            self.agents.remove(&oldest);
        }
        stale
    }

    fn lookup(&self, name: &str, now_secs: u64) -> Option<SeenAgent> {
        let seen = self.agents.get(&name.trim().to_lowercase())?;
        (now_secs.saturating_sub(seen.seen_secs) < FORGET_AFTER_SECS).then(|| seen.clone())
    }
}

fn read(path: &Path) -> HashMap<String, SeenAgent> {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// Write through a temp file and a rename, so a crash leaves the old record whole.
fn write(path: &Path, agents: &HashMap<String, SeenAgent>) {
    let Ok(json) = serde_json::to_vec(agents) else { return };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Where the record lives; none under test, so a test never reads or writes the real one.
fn record_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        Some(crate::backend::base::get_mux_data_dir().join("lan-seen-agents.json"))
    }
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

fn load_once(seen: &mut Seen) {
    if !seen.loaded {
        seen.loaded = true;
        if let Some(path) = record_path() {
            seen.agents = read(&path);
        }
    }
}

/// Note that `hostname` listed `names` at `now_secs`.
pub fn remember(names: &[String], hostname: &str, now_secs: u64) {
    let mut guard = seen().lock().unwrap_or_else(|e| e.into_inner());
    load_once(&mut guard);
    if guard.remember(names, hostname, now_secs) {
        if let Some(path) = record_path() {
            write(&path, &guard.agents);
        }
    }
}

/// Where `name` was last listed by a LAN peer, if one ever listed it (and not over 30 days ago).
pub fn lookup(name: &str) -> Option<SeenAgent> {
    let mut guard = seen().lock().unwrap_or_else(|e| e.into_inner());
    load_once(&mut guard);
    guard.lookup(name, agentmux_common::time::now_secs_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_listed_name_is_found_by_any_case_with_its_host() {
        let mut seen = Seen::default();
        assert!(seen.remember(&names(&["Maricon", " "]), "charlie", 10));
        assert_eq!(seen.lookup("MARICON", 11), Some(SeenAgent { hostname: "charlie".into(), seen_secs: 10 }));
        assert_eq!(seen.agents.len(), 1, "a blank name is not kept");
    }

    #[test]
    fn only_a_new_name_a_new_host_or_an_old_record_asks_for_a_write() {
        let mut seen = Seen::default();
        assert!(seen.remember(&names(&["Opaz"]), "charlie", 10), "new");
        assert!(!seen.remember(&names(&["Opaz"]), "charlie", 20), "same name, same host: no write");
        assert_eq!(seen.agents["opaz"].seen_secs, 10, "and the stored time stays");
        assert!(seen.remember(&names(&["opaz"]), "starpower", 30), "moved host");
        assert_eq!(seen.lookup("Opaz", 31).map(|s| s.hostname), Some("starpower".to_string()));
        assert!(seen.remember(&names(&["Opaz"]), "starpower", 30 + REFRESH_AFTER_SECS), "a day old");
    }

    #[test]
    fn a_name_not_listed_for_thirty_days_is_forgotten() {
        let mut seen = Seen::default();
        seen.remember(&names(&["Gone"]), "charlie", 10);
        assert!(seen.lookup("gone", 10 + FORGET_AFTER_SECS - 1).is_some());
        assert!(seen.lookup("gone", 10 + FORGET_AFTER_SECS).is_none());
    }

    #[test]
    fn past_the_cap_the_oldest_listing_goes() {
        let mut seen = Seen::default();
        for i in 0..MAX_SEEN + 5 {
            seen.remember(&[format!("agent{i}")], "h", i as u64);
        }
        assert_eq!(seen.agents.len(), MAX_SEEN);
        assert!(!seen.agents.contains_key("agent0") && seen.agents.contains_key(&format!("agent{}", MAX_SEEN + 4)));
    }

    #[test]
    fn the_record_survives_a_write_and_a_read_and_a_bad_file_reads_empty() {
        let dir = std::env::temp_dir().join(format!("seen-agents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lan-seen-agents.json");
        let mut seen = Seen::default();
        seen.remember(&names(&["Parlo"]), "charlie", 5);
        write(&path, &seen.agents);
        assert_eq!(read(&path).get("parlo"), Some(&SeenAgent { hostname: "charlie".into(), seen_secs: 5 }));
        std::fs::write(&path, b"not json").unwrap();
        assert!(read(&path).is_empty());
        assert!(read(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
