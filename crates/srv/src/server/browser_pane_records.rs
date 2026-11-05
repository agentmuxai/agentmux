// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The browser panes an agent drives, and the sites a limited one may go to, kept across an srv
//! restart (docs/specs/SPEC_BROWSER_PANE_RECORDS_SURVIVE_RESTART_2026_11_04.md).
//!
//! Who owns a pane (`browser_owner`) and which sites its chain is limited to (`browser_allowlist`)
//! are srv's own records, held in memory. Without a copy on disk a restart leaves a limited,
//! agent-owned pane unlimited and unowned while its block still shows both. A file in the config
//! folder holds the copy: written whenever either record changes, read once at startup.
//!
//! The file is keyed by block id, and a duplicated pane is a new block with a new id, so a copy of a
//! pane never inherits anything. A record is taken back only for a block that is still a browser
//! pane whose `browser:owner_agent` (which only srv writes, and a person's Take over clears) names
//! the same agent: the file alone grants nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::backend::obj::Block;
use crate::server::{browser_allowlist, browser_owner, AppState};

/// The file the records are kept in, in the config folder.
const FILE_NAME: &str = "browser-pane-records.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Records {
    /// Pane → the agent that owns it.
    #[serde(default)]
    pub owners: BTreeMap<String, String>,
    /// Limited pane → the root pane of its chain.
    #[serde(default)]
    pub members: BTreeMap<String, String>,
    /// Root pane → the chain's site list.
    #[serde(default)]
    pub lists: BTreeMap<String, Vec<String>>,
}

fn path() -> PathBuf {
    crate::backend::base::get_mux_config_dir().join(FILE_NAME)
}

/// What srv holds now.
fn current() -> Records {
    let (members, lists) = browser_allowlist::chains();
    Records { owners: browser_owner::all_owners(), members, lists }
}

/// The part of `saved` that is still true: a pane that is still there as a browser pane its agent
/// still drives, and the limits of the panes that remain. `block_of` reads a block by id.
fn still_true(saved: Records, block_of: impl Fn(&str) -> Option<Block>) -> Records {
    let owners: BTreeMap<String, String> = saved
        .owners
        .into_iter()
        .filter(|(pane, agent)| {
            block_of(pane).is_some_and(|b| {
                let meta_str = |k: &str| b.meta.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                meta_str("view") == "browser" && meta_str(browser_owner::OWNER_META_KEY).eq_ignore_ascii_case(agent)
            })
        })
        .collect();
    let members: BTreeMap<String, String> = saved.members.into_iter().filter(|(pane, _)| owners.contains_key(pane)).collect();
    let lists = saved.lists.into_iter().filter(|(root, _)| members.values().any(|r| r == root)).collect();
    Records { owners, members, lists }
}

/// Read the file and put back what is still true. Called once at startup, before anything
/// changes the records, so the first save doesn't replace the file with an empty copy.
pub(crate) fn restore(state: &AppState) {
    let saved = match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice::<Records>(&bytes).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "[browser-records] the saved records are unreadable; panes start unowned and unlimited");
            Records::default()
        }),
        Err(_) => Records::default(),
    };
    let kept = still_true(saved, |pane| state.mstore.get::<Block>(pane).ok().flatten());
    for (pane, agent) in &kept.owners {
        browser_owner::restore(pane, agent);
    }
    for (pane, root) in &kept.members {
        if let Some(list) = kept.lists.get(root) {
            browser_allowlist::restore(pane, root, list.clone());
        }
    }
    tracing::info!(owned = kept.owners.len(), limited = kept.members.len(), "[browser-records] restored");
}

/// Write the records to the file, unless they are what the last write left there.
pub(crate) fn save() {
    static LAST: OnceLock<Mutex<Option<Records>>> = OnceLock::new();
    let now = current();
    let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    if last.as_ref() == Some(&now) {
        return;
    }
    let bytes = match serde_json::to_vec_pretty(&now) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[browser-records] couldn't encode the records");
            return;
        }
    };
    match agentmux_common::settings_file::write_atomic(&path(), &bytes) {
        Ok(()) => *last = Some(now),
        Err(e) => tracing::warn!(error = %e, "[browser-records] couldn't save the records"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(view: &str, owner: Option<&str>) -> Block {
        let mut b = Block::default();
        b.meta.insert("view".to_string(), json!(view));
        if let Some(o) = owner {
            b.meta.insert(browser_owner::OWNER_META_KEY.to_string(), json!(o));
        }
        b
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn a_record_comes_back_only_for_a_pane_still_there_and_still_the_agents() {
        let saved = Records {
            owners: [pair("kept", "lark"), pair("taken-over", "lark"), pair("gone", "lark"), pair("other-agent", "lark"), pair("not-a-browser", "lark")].into(),
            members: [pair("kept", "root"), pair("taken-over", "root2"), pair("gone", "root3")].into(),
            lists: [("root".to_string(), vec!["https://a.example".to_string()]), ("root2".to_string(), vec![]), ("root3".to_string(), vec![])].into(),
        };
        let blocks: BTreeMap<&str, Block> = [
            ("kept", block("browser", Some("Lark"))),
            ("taken-over", block("browser", None)),
            ("other-agent", block("browser", Some("korp"))),
            ("not-a-browser", block("term", Some("lark"))),
        ]
        .into();
        let back = still_true(saved, |p| blocks.get(p).cloned());
        assert_eq!(back.owners, [pair("kept", "lark")].into());
        assert_eq!(back.members, [pair("kept", "root")].into());
        assert_eq!(back.lists.keys().collect::<Vec<_>>(), vec!["root"]);
    }

    #[test]
    fn a_copy_of_a_pane_has_its_meta_but_no_record() {
        let saved = Records { owners: [pair("original", "lark")].into(), ..Default::default() };
        // The copy carries the same meta under a new id; only the original has a record.
        let blocks: BTreeMap<&str, Block> = [("original", block("browser", Some("lark"))), ("copy", block("browser", Some("lark")))].into();
        let back = still_true(saved, |p| blocks.get(p).cloned());
        assert!(back.owners.contains_key("original") && !back.owners.contains_key("copy"));
    }

    #[test]
    fn a_limited_popup_outlives_its_root_across_a_restart() {
        let saved = Records {
            owners: [pair("popup", "lark")].into(),
            members: [pair("popup", "root-closed")].into(),
            lists: [("root-closed".to_string(), vec!["https://a.example".to_string()])].into(),
        };
        let back = still_true(saved, |p| (p == "popup").then(|| block("browser", Some("lark"))));
        assert_eq!(back.lists.get("root-closed").map(Vec::len), Some(1));
    }

    #[test]
    fn restored_records_drive_the_owner_and_the_site_list() {
        browser_owner::restore("rec-popup", "lark");
        browser_allowlist::restore("rec-popup", "rec-root", vec!["https://a.example".to_string()]);
        assert_eq!(browser_owner::owner_of("rec-popup").as_deref(), Some("lark"));
        assert_eq!(browser_allowlist::list_for("rec-popup"), Some(vec!["https://a.example".to_string()]));
        assert!(browser_allowlist::refuse_agent_navigation("rec-popup", "https://b.example/x").is_some());
        let now = current();
        assert_eq!(now.owners.get("rec-popup").map(String::as_str), Some("lark"));
        assert_eq!(now.members.get("rec-popup").map(String::as_str), Some("rec-root"));
        // A Take over or a closed pane ends both.
        browser_owner::forget("rec-popup");
        assert!(browser_allowlist::drop_pane("rec-popup"));
        assert!(browser_owner::owner_of("rec-popup").is_none() && browser_allowlist::list_for("rec-popup").is_none());
        assert!(!current().lists.contains_key("rec-root"));
    }

    #[test]
    fn the_records_survive_the_file() {
        let r = Records {
            owners: [pair("a", "lark")].into(),
            members: [pair("a", "r")].into(),
            lists: [("r".to_string(), vec!["https://a.example".to_string()])].into(),
        };
        let back: Records = serde_json::from_slice(&serde_json::to_vec_pretty(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        // An older or partial file reads as what it has.
        assert_eq!(serde_json::from_str::<Records>("{}").unwrap(), Records::default());
    }
}
