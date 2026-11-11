// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's "always on this site" for a page's camera, microphone, location or notifications in a
//! pane an agent drives (docs/reports/REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md §4.5, medium level).
//!
//! A row names one agent, one site and the kinds it may use there. It is a setting: it lives in a file in
//! the config folder, survives restarts, is listed in Settings and revoked there. It is not an Auto-approve
//! grant (`browser_standing`): those lift the wait for committing clicks, and a camera must not bring that
//! along, so these rows are a list of their own.
//!
//! Only the person writes rows, from the Settings window (`app_api/browser_grants.rs` checks that). With
//! a row, `browser_permission` allows the page's request at once, with no countdown, and writes one Audit
//! row flagged `standing` for the first use in a pane.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::server::AppState;

/// The file the rows are kept in, in the config folder.
const FILE_NAME: &str = "browser-site-permissions.json";

/// Most rows kept: a list this long is a mistake, not a use.
const MAX_ROWS: usize = 200;

/// What a row can carry, in the words `browser_permission` names them.
pub(crate) const KINDS: [&str; 4] = ["camera", "microphone", "location", "notifications"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Row {
    pub id: String,
    /// The agent's id (its slug), as the pane's owner is known.
    pub agent: String,
    /// The one site, in `allowed_origins` normal form (`https://meet.example.com`).
    pub site: String,
    pub kinds: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub granted_at_ms: i64,
}

/// The rows, and where they are kept (None: nowhere, for tests).
struct Store {
    path: Option<PathBuf>,
    rows: Vec<Row>,
}

impl Store {
    fn open(path: PathBuf) -> Store {
        let rows = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Vec<Row>>(&bytes).unwrap_or_else(|e| {
                // A file nobody can read allows nothing: the person sees an empty list.
                tracing::warn!(error = %e, "[browser-site-permissions] the file is unreadable; none are in force");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        Store { path: Some(path), rows }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        match serde_json::to_vec_pretty(&self.rows) {
            Ok(bytes) => {
                if let Err(e) = crate::server::browser_standing::write_atomic(path, &bytes) {
                    tracing::warn!(error = %e, "[browser-site-permissions] couldn't save the rows");
                }
            }
            Err(e) => tracing::warn!(error = %e, "[browser-site-permissions] couldn't encode the rows"),
        }
    }
}

fn store() -> std::sync::MutexGuard<'static, Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Store::open(crate::backend::base::get_mux_config_dir().join(FILE_NAME))))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// `kinds` checked: at least one, each a known kind, listed once in the fixed order.
fn parse_kinds(asked: &[String]) -> Result<Vec<String>, String> {
    if let Some(bad) = asked.iter().find(|k| !KINDS.iter().any(|known| known.eq_ignore_ascii_case(k.trim()))) {
        return Err(format!("unknown permission {bad:?}: choose from {}", KINDS.join(", ")));
    }
    let kinds: Vec<String> = KINDS
        .iter()
        .filter(|known| asked.iter().any(|k| known.eq_ignore_ascii_case(k.trim())))
        .map(|k| k.to_string())
        .collect();
    if kinds.is_empty() {
        return Err("choose what the site may use".to_string());
    }
    Ok(kinds)
}

/// The one site, in normal form. A wildcard is refused: a row names one site.
fn parse_site(site: &str) -> Result<String, String> {
    let origins = agentmux_common::allowed_origins::parse_list(&[site.to_string()])?;
    match origins.as_slice() {
        [one] if !one.contains("://*.") => Ok(one.clone()),
        [one] => Err(format!("{one:?} names every subdomain: name one site (https://meet.example.com)")),
        _ => Err("name one site".to_string()),
    }
}

/// Add the row for `agent` on `site`, or replace its kinds.
fn upsert_in(rows: &mut Vec<Row>, agent: &str, site: &str, kinds: &[String], now_ms: i64) -> Result<Row, String> {
    let agent = agent.trim().to_ascii_lowercase();
    if agent.is_empty() {
        return Err("name the agent".to_string());
    }
    let site = parse_site(site)?;
    let kinds = parse_kinds(kinds)?;
    if let Some(row) = rows.iter_mut().find(|r| r.agent == agent && r.site == site) {
        row.kinds = kinds;
        return Ok(row.clone());
    }
    if rows.len() >= MAX_ROWS {
        return Err(format!("there are already {MAX_ROWS} site permissions: revoke some first"));
    }
    let row = Row { id: uuid::Uuid::new_v4().to_string(), agent, site, kinds, granted_at_ms: now_ms };
    rows.push(row.clone());
    Ok(row)
}

/// The row that lets `agent` use all of `kinds` on `site`, if one does.
fn covering<'a>(rows: &'a [Row], agent: &str, site: &str, kinds: &[&str]) -> Option<&'a Row> {
    rows.iter().find(|r| r.agent.eq_ignore_ascii_case(agent) && r.site == site && kinds.iter().all(|k| r.kinds.iter().any(|have| have == k)))
}

pub(crate) fn list() -> Vec<Row> {
    store().rows.clone()
}

/// Add a row, or change the kinds of the one for this agent and site. The caller has checked that the
/// person is the one asking.
pub(crate) fn upsert(state: &AppState, agent: &str, site: &str, kinds: &[String]) -> Result<Row, String> {
    let row = {
        let mut s = store();
        let row = upsert_in(&mut s.rows, agent, site, kinds, chrono::Utc::now().timestamp_millis())?;
        s.save();
        row
    };
    tracing::info!(agent = %row.agent, site = %row.site, kinds = ?row.kinds, "[browser-site-permissions] a site permission was set");
    state.broker.publish_event::<crate::backend::mps_events::BrowserGrantsChanged>(&(), vec![], 0);
    Ok(row)
}

/// Remove row `id`. Returns whether it was there. The next request from the page asks again.
pub(crate) fn revoke(state: &AppState, id: &str) -> bool {
    let removed = {
        let mut s = store();
        let before = s.rows.len();
        s.rows.retain(|r| r.id != id);
        let gone = s.rows.len() != before;
        if gone {
            s.save();
        }
        gone
    };
    if removed {
        state.broker.publish_event::<crate::backend::mps_events::BrowserGrantsChanged>(&(), vec![], 0);
        tracing::info!(row = %id, "[browser-site-permissions] a site permission was revoked");
    }
    removed
}

/// Whether the person has allowed `agent` all of `kinds` on `site` for good.
pub(crate) fn allowed(agent: &str, site: &str, kinds: &[&str]) -> bool {
    covering(&store().rows, agent, site, kinds).is_some()
}

/// Whether this is the first use of the permission in `target` since AgentMux started, which is the one
/// that earns an Audit row: a page asking again adds none. Kept small: it forgets all when it fills.
pub(crate) fn first_use(agent: &str, target: &str, site: &str, kinds: &[&str]) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    if seen.len() >= 1024 {
        seen.clear();
    }
    seen.insert(format!("{agent}|{target}|{site}|{}", kinds.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_row_covers_its_agent_on_its_site_for_the_kinds_it_carries_only() {
        let mut rows = Vec::new();
        upsert_in(&mut rows, "Posa", "https://meet.example.com", &kinds(&["microphone", "camera"]), 1).unwrap();
        assert_eq!(rows[0].kinds, ["camera", "microphone"], "listed in the fixed order");
        assert!(covering(&rows, "posa", "https://meet.example.com", &["camera"]).is_some());
        assert!(covering(&rows, "posa", "https://meet.example.com", &["camera", "microphone"]).is_some());
        assert!(covering(&rows, "posa", "https://meet.example.com", &["location"]).is_none(), "not carried");
        assert!(covering(&rows, "lazo", "https://meet.example.com", &["camera"]).is_none(), "another agent");
        assert!(covering(&rows, "posa", "https://other.example.com", &["camera"]).is_none(), "another site");
        assert!(covering(&rows, "posa", "https://meet.example.com:8443", &["camera"]).is_none(), "another port is another site");
    }

    #[test]
    fn setting_a_row_again_replaces_its_kinds_instead_of_adding_one() {
        let mut rows = Vec::new();
        let first = upsert_in(&mut rows, "posa", "https://meet.example.com", &kinds(&["camera"]), 1).unwrap();
        let again = upsert_in(&mut rows, "posa", "https://meet.example.com", &kinds(&["notifications"]), 2).unwrap();
        assert_eq!((rows.len(), first.id.clone()), (1, again.id));
        assert_eq!(rows[0].kinds, ["notifications"]);
    }

    #[test]
    fn an_unknown_kind_a_wildcard_and_an_empty_choice_are_refused() {
        let mut rows = Vec::new();
        let site = "https://meet.example.com";
        assert!(upsert_in(&mut rows, "posa", site, &kinds(&["screen"]), 1).unwrap_err().contains("screen"));
        assert!(upsert_in(&mut rows, "posa", site, &[], 1).is_err());
        assert!(upsert_in(&mut rows, "posa", "https://*.example.com", &kinds(&["camera"]), 1).is_err(), "a row names one site");
        assert!(upsert_in(&mut rows, " ", site, &kinds(&["camera"]), 1).is_err());
        assert!(rows.is_empty());
    }

    #[test]
    fn the_list_is_capped() {
        let mut rows = Vec::new();
        for i in 0..MAX_ROWS {
            upsert_in(&mut rows, "posa", &format!("https://s{i}.example.com"), &kinds(&["camera"]), 1).unwrap();
        }
        assert!(upsert_in(&mut rows, "posa", "https://one-more.example.com", &kinds(&["camera"]), 1).is_err());
        assert!(upsert_in(&mut rows, "posa", "https://s0.example.com", &kinds(&["location"]), 1).is_ok(), "changing a row is not adding one");
    }

    #[test]
    fn rows_survive_a_restart_and_an_unreadable_file_allows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut s = Store { path: Some(path.clone()), rows: Vec::new() };
        upsert_in(&mut s.rows, "posa", "https://meet.example.com", &kinds(&["camera"]), 1).unwrap();
        s.save();
        assert_eq!(Store::open(path.clone()).rows, s.rows);
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(Store::open(path).rows.is_empty());
    }

    #[test]
    fn only_the_first_use_in_a_pane_is_audited() {
        let site = format!("https://{}.example", uuid::Uuid::new_v4().simple());
        assert!(first_use("posa", "pane-1", &site, &["camera"]));
        assert!(!first_use("posa", "pane-1", &site, &["camera"]));
        assert!(first_use("posa", "pane-2", &site, &["camera"]), "another pane is another use");
    }
}
