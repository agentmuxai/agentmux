// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's standing grants: "this agent may do these things on this
//! site, until I say otherwise"
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §3).
//!
//! A row names one agent and one site (with, for a preset, the frames of other
//! sites that belong to it) and the powers it carries. It is a
//! setting: it lives in a file in the config folder and survives restarts. A
//! row grants nothing by itself. When its agent works on a pane showing a
//! site the row names, [`activate_at`] makes a pane grant from it (the strip,
//! Stop, Pause, Take over and the idle rule are the pane grant's), so the
//! agent never has to ask again and the person's controls are the ones they
//! already know.
//!
//! Only the person writes rows: through the Settings window
//! (`app_api/browser_grants.rs`, which checks that), or by answering a
//! request banner with "Always". No agent route reaches this module.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::server::browser_bypass as bypass;
use crate::server::browser_powers::{self, Power};
use crate::server::AppState;

/// The file the rows are kept in, in the config folder.
const FILE_NAME: &str = "browser-standing-grants.json";

/// Most rows kept: a list this long is a mistake, not a use.
const MAX_ROWS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Row {
    pub id: String,
    /// The agent's id (its slug), as `request_auto_approve` sees it.
    pub agent: String,
    /// The sites, in `allowed_origins` normal form: the one the person sees
    /// first, then any whose frames belong to it (Cloud Shell's terminal runs
    /// in a frame of another site than the page that shows it).
    pub origins: Vec<String>,
    /// The wire names of the powers, committing clicks always among them.
    pub powers: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub granted_at_ms: i64,
}

impl Row {
    /// The powers as the grant module knows them.
    pub(crate) fn powers(&self) -> Vec<Power> {
        browser_powers::parse(Some(&self.powers)).unwrap_or_else(|_| vec![Power::Clicks])
    }
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
                // A file nobody can read grants nothing: the person sees an empty list.
                tracing::warn!(error = %e, "[browser-standing] the standing grants file is unreadable; none are in force");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        Store { path: Some(path), rows }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let bytes = match serde_json::to_vec_pretty(&self.rows) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "[browser-standing] couldn't encode the standing grants");
                return;
            }
        };
        if let Err(e) = write_atomic(path, &bytes) {
            tracing::warn!(error = %e, "[browser-standing] couldn't save the standing grants");
        }
    }
}

/// Write `bytes` to `path` so a reader never sees half a file: a temp file in
/// the same folder, flushed, then renamed over it.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no folder to write in"))?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::Builder::new().prefix(".standing-grants-").suffix(".json").tempfile_in(parent)?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn store() -> std::sync::MutexGuard<'static, Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Store::open(crate::backend::base::get_mux_config_dir().join(FILE_NAME))))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// The row of `rows` that lets `agent` work on `url`, if one does. A page with
/// no web origin (`about:blank`, `data:`) is never covered.
fn find<'a>(rows: &'a [Row], agent: &str, url: &str) -> Option<&'a Row> {
    agentmux_common::allowed_origins::target_origin(url)?;
    rows.iter()
        .find(|r| r.agent.eq_ignore_ascii_case(agent) && agentmux_common::allowed_origins::allows(&r.origins, url))
}

/// Add or replace the row for `agent` on `sites` (the first names the row).
/// A wildcard (`*.example.com`) is only for a preset or a banner the person
/// answered (`wildcard_ok`): a row the person types in names one site.
fn upsert_in(
    rows: &mut Vec<Row>,
    agent: &str,
    sites: &[String],
    powers: &[String],
    wildcard_ok: bool,
    now_ms: i64,
) -> Result<Row, String> {
    let agent = agent.trim().to_ascii_lowercase();
    if agent.is_empty() {
        return Err("name the agent".to_string());
    }
    if sites.is_empty() {
        return Err("name the site".to_string());
    }
    let origins = agentmux_common::allowed_origins::parse_list(sites)?;
    if !wildcard_ok {
        if let Some(wild) = origins.iter().find(|o| o.contains("://*.")) {
            return Err(format!("{wild:?} names every subdomain: a standing grant names one site (https://shell.example.com)"));
        }
    }
    let powers: Vec<String> = browser_powers::names(&browser_powers::parse(Some(powers))?).iter().map(|n| n.to_string()).collect();
    if let Some(row) = rows.iter_mut().find(|r| r.agent == agent && r.origins.first() == origins.first()) {
        row.origins = origins;
        row.powers = powers;
        return Ok(row.clone());
    }
    if rows.len() >= MAX_ROWS {
        return Err(format!("there are already {MAX_ROWS} standing grants: revoke some first"));
    }
    let row = Row { id: uuid::Uuid::new_v4().to_string(), agent, origins, powers, granted_at_ms: now_ms };
    rows.push(row.clone());
    Ok(row)
}

/// A console the person can switch on for an agent with one click.
pub(crate) struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    /// The page's site first, then the sites whose frames belong to it.
    pub sites: &'static [&'static str],
    /// What the person is told beside it.
    pub note: &'static str,
}

/// The presets. The sites are the ones the console's own documentation names;
/// each is to be checked against the real console in a signed-in run (the spec's
/// Phase 0 live check), and AWS CloudShell and GitHub Codespaces wait for that
/// check, because their hostnames vary by region and by codespace.
pub(crate) const PRESETS: [Preset; 2] = [
    Preset {
        id: "google-cloud-shell",
        label: "Google Cloud Shell",
        sites: &["https://shell.cloud.google.com", "https://ssh.cloud.google.com"],
        note: "Not yet checked against a signed-in Cloud Shell: if the terminal doesn't work, tell the person who set it up.",
    },
    Preset {
        id: "azure-cloud-shell",
        label: "Azure Cloud Shell",
        sites: &["https://shell.azure.com"],
        note: "Not yet checked against a signed-in Cloud Shell: if the terminal doesn't work, tell the person who set it up.",
    },
];

/// Turn a preset on for `agent` with the terminal power (a preset is for
/// consoles).
pub(crate) fn enable_preset(state: &AppState, preset: &str, agent: &str) -> Result<Row, String> {
    let p = PRESETS.iter().find(|p| p.id == preset).ok_or_else(|| format!("unknown preset {preset:?}"))?;
    let sites: Vec<String> = p.sites.iter().map(|s| s.to_string()).collect();
    upsert(state, agent, &sites, &["terminal".to_string()], true)
}

/// Every row, oldest first.
pub(crate) fn list() -> Vec<Row> {
    store().rows.clone()
}

/// Add a row, or change the powers of the one for this agent and site. The
/// caller has checked that the person is the one asking. A changed row ends the
/// pane grants made from it, which are made again from the new row on the
/// agent's next call.
pub(crate) fn upsert(state: &AppState, agent: &str, sites: &[String], powers: &[String], wildcard_ok: bool) -> Result<Row, String> {
    let (row, existed) = {
        let mut s = store();
        let had: Vec<String> = s.rows.iter().map(|r| r.id.clone()).collect();
        let row = upsert_in(&mut s.rows, agent, sites, powers, wildcard_ok, chrono::Utc::now().timestamp_millis())?;
        s.save();
        let existed = had.contains(&row.id);
        (row, existed)
    };
    // A row that already existed may have held grants with other powers.
    if existed {
        bypass::end_standing(state, &row.id, "you changed what it may do");
    }
    tracing::info!(agent = %row.agent, origins = ?row.origins, powers = ?row.powers, "[browser-standing] a standing grant was set");
    Ok(row)
}

/// The person's "Always" on a request banner: a row for `agent` on the sites
/// the banner named, with `powers` added to whatever it already carried (a
/// banner that shows fewer powers than a row has never narrows the row).
pub(crate) fn add_powers(state: &AppState, agent: &str, sites: &[String], powers: &[String]) -> Result<Row, String> {
    let origins = agentmux_common::allowed_origins::parse_list(sites)?;
    let mut all: Vec<String> = store()
        .rows
        .iter()
        .find(|r| r.agent.eq_ignore_ascii_case(agent.trim()) && r.origins.first() == origins.first())
        .map(|r| r.powers.clone())
        .unwrap_or_default();
    all.extend(powers.iter().cloned());
    upsert(state, agent, sites, &all, true)
}

/// Change the powers of row `id`.
pub(crate) fn set_powers(state: &AppState, id: &str, powers: &[String]) -> Result<Row, String> {
    let powers: Vec<String> = browser_powers::names(&browser_powers::parse(Some(powers))?).iter().map(|n| n.to_string()).collect();
    let row = {
        let mut s = store();
        let row = s.rows.iter_mut().find(|r| r.id == id).ok_or_else(|| "that standing grant is gone".to_string())?;
        row.powers = powers;
        let row = row.clone();
        s.save();
        row
    };
    bypass::end_standing(state, &row.id, "you changed what it may do");
    Ok(row)
}

/// Remove row `id` and end the pane grants made from it, at once: an
/// in-flight call ends and the strip comes down. Returns whether the row was
/// there.
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
        bypass::end_standing(state, id, "you revoked it");
        tracing::info!(row = %id, "[browser-standing] a standing grant was revoked");
    }
    removed
}

/// The pane grant for `agent` on `pane`, made from a standing row that names
/// the site `url` (the address the host read from the page itself). None: no
/// row says so. A grant `agent` already holds there is not replaced.
pub(crate) fn activate_at(state: &AppState, pane: &str, agent: &str, url: &str) -> Option<bypass::Grant> {
    if let Some(g) = bypass::active_for(pane, agent) {
        return Some(g);
    }
    let row = find(&store().rows, agent, url)?.clone();
    Some(bypass::grant_standing(state, pane, agent, row.origins.clone(), row.powers(), &row.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sites(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn rows() -> Vec<Row> {
        let mut r = Vec::new();
        upsert_in(&mut r, "Naki-2", &sites(&["shell.cloud.google.com"]), &["terminal".to_string()], false, 1).unwrap();
        r
    }

    #[test]
    fn a_row_covers_its_agent_on_its_site_only() {
        let r = rows();
        assert_eq!(r[0].origins, vec!["https://shell.cloud.google.com"]);
        assert_eq!(r[0].agent, "naki-2");
        assert!(find(&r, "naki-2", "https://shell.cloud.google.com/?x=1").is_some());
        assert!(find(&r, "NAKI-2", "https://shell.cloud.google.com/").is_some());
        // Another agent, another site, another scheme, another port, no web origin at all.
        assert!(find(&r, "agent1", "https://shell.cloud.google.com/").is_none());
        assert!(find(&r, "naki-2", "https://evil.example.com/").is_none());
        assert!(find(&r, "naki-2", "http://shell.cloud.google.com/").is_none());
        assert!(find(&r, "naki-2", "https://shell.cloud.google.com:8443/").is_none());
        assert!(find(&r, "naki-2", "about:blank").is_none());
        assert!(find(&r, "naki-2", "data:text/html,hi").is_none());
    }

    #[test]
    fn clicks_are_always_a_power_of_a_row_and_an_unknown_power_is_refused() {
        let r = rows();
        assert_eq!(r[0].powers, vec!["clicks", "terminal"]);
        assert_eq!(r[0].powers(), vec![Power::Clicks, Power::Terminal]);
        let mut v = Vec::new();
        assert!(upsert_in(&mut v, "a", &sites(&["example.com"]), &["root".to_string()], false, 1).is_err());
    }

    #[test]
    fn a_typed_row_names_one_site_and_only_a_preset_or_a_banner_may_use_a_wildcard() {
        let mut v = Vec::new();
        assert!(upsert_in(&mut v, "a", &sites(&["*.github.dev"]), &[], false, 1).is_err());
        let row = upsert_in(&mut v, "a", &sites(&["*.github.dev"]), &[], true, 1).unwrap();
        assert_eq!(row.origins, vec!["https://*.github.dev"]);
        assert!(upsert_in(&mut v, "", &sites(&["example.com"]), &[], false, 1).is_err());
        assert!(upsert_in(&mut v, "a", &sites(&["not a site"]), &[], false, 1).is_err());
        assert!(upsert_in(&mut v, "a", &[], &[], false, 1).is_err());
    }

    #[test]
    fn a_row_with_frames_covers_the_page_and_the_frame_but_nothing_else() {
        let mut v = Vec::new();
        upsert_in(&mut v, "a", &sites(&["shell.example.com", "ssh.example.com"]), &["terminal".to_string()], false, 1).unwrap();
        assert!(find(&v, "a", "https://shell.example.com/").is_some());
        assert!(find(&v, "a", "https://ssh.example.com/proxy").is_some());
        assert!(find(&v, "a", "https://www.example.com/").is_none());
    }

    #[test]
    fn setting_a_row_again_replaces_its_powers_instead_of_adding_one() {
        let mut v = rows();
        let first = v[0].id.clone();
        let again = upsert_in(&mut v, "naki-2", &sites(&["https://shell.cloud.google.com"]), &[], false, 9).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(again.id, first);
        assert_eq!(again.powers, vec!["clicks"]);
        assert_eq!(again.granted_at_ms, 1, "the day it was first granted is kept");
    }

    #[test]
    fn every_preset_is_a_set_of_sites_a_row_accepts() {
        let mut v = Vec::new();
        for p in PRESETS {
            let sites: Vec<String> = p.sites.iter().map(|s| s.to_string()).collect();
            let row = upsert_in(&mut v, "naki-2", &sites, &["terminal".to_string()], true, 1).unwrap();
            assert_eq!(row.origins.len(), p.sites.len(), "{}", p.id);
            assert!(!p.note.is_empty() && !p.label.is_empty());
        }
    }

    #[test]
    fn the_list_is_capped() {
        let mut v = Vec::new();
        for i in 0..MAX_ROWS {
            upsert_in(&mut v, "a", &sites(&[&format!("site{i}.example.com")]), &[], false, 1).unwrap();
        }
        assert!(upsert_in(&mut v, "a", &sites(&["one-more.example.com"]), &[], false, 1).is_err());
    }

    #[test]
    fn rows_survive_a_restart_and_an_unreadable_file_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut s = Store { path: Some(path.clone()), rows: rows() };
        s.save();
        let reopened = Store::open(path.clone());
        assert_eq!(reopened.rows, s.rows);
        s.rows.clear();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(Store::open(path).rows.is_empty());
    }
}
