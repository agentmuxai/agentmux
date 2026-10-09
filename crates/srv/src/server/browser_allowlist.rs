// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sites an agent limited its browser pane to with
//! `OpenBrowser({allowed_origins})`
//! (docs/specs/SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md).
//!
//! One list per chain of panes: the pane `OpenBrowser` opened (the root) and
//! every popup pane opened from it, or from those. Like the owner map, this
//! is srv's own record, in memory: the block's `browser:allowed_origins`
//! meta key mirrors it for the pane's header, and clients can't write it. The
//! host keeps a copy of every member's list, pushed with the owned-pane set
//! (`ui_handlers::spawn_owned_panes_sync`), and stops navigations off it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::server::AppState;

/// Block meta key mirroring a pane's list, for its header.
pub(crate) const ALLOWED_META_KEY: &str = "browser:allowed_origins";

#[derive(Default)]
struct Lists {
    /// Root pane → its chain's list (normal form, `allowed_origins::parse`).
    lists: HashMap<String, Vec<String>>,
    /// Every member pane, the root included → its root.
    members: HashMap<String, String>,
}

fn lists() -> &'static Mutex<Lists> {
    static L: OnceLock<Mutex<Lists>> = OnceLock::new();
    L.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Lists> {
    lists().lock().unwrap_or_else(|p| p.into_inner())
}

/// Limit `root` (a pane `OpenBrowser` just opened) to `list`.
pub(crate) fn set(root: &str, list: Vec<String>) {
    let mut l = lock();
    l.lists.insert(root.to_string(), list);
    l.members.insert(root.to_string(), root.to_string());
    drop(l);
    crate::server::browser_owner::changed().notify_one();
}

/// `pane`, a popup pane `opener` just opened, joins `opener`'s chain, if it
/// has a list. Returns the list it now shares.
pub(crate) fn join(pane: &str, opener: &str) -> Option<Vec<String>> {
    let mut l = lock();
    let root = l.members.get(opener)?.clone();
    let list = l.lists.get(&root)?.clone();
    l.members.insert(pane.to_string(), root);
    drop(l);
    crate::server::browser_owner::changed().notify_one();
    Some(list)
}

/// `pane`'s list, if its chain has one.
pub(crate) fn list_for(pane: &str) -> Option<Vec<String>> {
    let l = lock();
    let root = l.members.get(pane)?;
    l.lists.get(root).cloned()
}

/// Add `origin` to `pane`'s chain's list: the person allowed it. Returns the
/// list after, and the chain's members, whose header shows it.
pub(crate) fn allow(pane: &str, origin: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut l = lock();
    let root = l.members.get(pane)?.clone();
    let list = l.lists.get_mut(&root)?;
    if !list.iter().any(|o| o == origin) {
        list.push(origin.to_string());
    }
    let list = list.clone();
    let members = l.members.iter().filter(|(_, r)| **r == root).map(|(m, _)| m.clone()).collect();
    drop(l);
    crate::server::browser_owner::changed().notify_one();
    Some((list, members))
}

/// `pane` was closed or taken over. A member just leaves its chain; the root
/// takes the whole chain's list with it, since the agent's hold on the chain
/// ends with the root's. Returns the panes that no longer have a list.
pub(crate) fn drop_pane(pane: &str) -> Vec<String> {
    let mut l = lock();
    let Some(root) = l.members.get(pane).cloned() else { return Vec::new() };
    let dropped: Vec<String> = if root == pane {
        l.lists.remove(&root);
        let gone: Vec<String> = l.members.iter().filter(|(_, r)| **r == root).map(|(m, _)| m.clone()).collect();
        for m in &gone {
            l.members.remove(m);
        }
        gone
    } else {
        l.members.remove(pane);
        vec![pane.to_string()]
    };
    drop(l);
    crate::server::browser_owner::changed().notify_one();
    dropped
}

/// Every member pane's list, for the host.
pub(crate) fn snapshot() -> HashMap<String, Vec<String>> {
    let l = lock();
    l.members
        .iter()
        .filter_map(|(m, r)| l.lists.get(r).map(|list| (m.clone(), list.clone())))
        .collect()
}

/// Write `list` (or nothing) to each of `panes`' headers.
pub(crate) fn publish(state: &AppState, panes: &[String], list: Option<&[String]>) {
    let value = list.map(|l| json!(l)).unwrap_or(Value::Null);
    for pane in panes {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert(ALLOWED_META_KEY.to_string(), value.clone());
        if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, pane, &meta) {
            tracing::debug!(pane = %pane, error = %e, "[browser-allowlist] pane header not updated");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_shares_one_list_and_the_root_takes_it_along() {
        set("al-root", vec!["https://a.example".into()]);
        assert_eq!(join("al-popup", "al-root").unwrap(), vec!["https://a.example"]);
        assert_eq!(join("al-popup2", "al-popup").unwrap(), vec!["https://a.example"]);
        // Allowing on any member adds to the whole chain.
        let (list, mut members) = allow("al-popup2", "https://b.example").unwrap();
        members.sort();
        assert_eq!(list, vec!["https://a.example", "https://b.example"]);
        assert_eq!(members, vec!["al-popup", "al-popup2", "al-root"]);
        assert_eq!(list_for("al-root").unwrap().len(), 2);
        // A member leaving keeps the rest.
        assert_eq!(drop_pane("al-popup"), vec!["al-popup"]);
        assert!(list_for("al-popup").is_none() && list_for("al-popup2").is_some());
        // The root leaving ends the chain.
        let mut gone = drop_pane("al-root");
        gone.sort();
        assert_eq!(gone, vec!["al-popup2", "al-root"]);
        assert!(list_for("al-popup2").is_none());
        assert!(!snapshot().keys().any(|k| k.starts_with("al-")));
    }

    #[test]
    fn a_pane_without_a_list_has_nothing_to_join_or_allow() {
        assert!(join("al-x-popup", "al-x-unlisted").is_none());
        assert!(allow("al-x-unlisted", "https://b.example").is_none());
        assert!(drop_pane("al-x-unlisted").is_empty());
    }

    #[test]
    fn the_snapshot_lists_every_member() {
        set("al-s-root", vec!["https://a.example".into()]);
        join("al-s-popup", "al-s-root");
        let snap = snapshot();
        assert_eq!(snap.get("al-s-root"), Some(&vec!["https://a.example".to_string()]));
        assert_eq!(snap.get("al-s-popup"), Some(&vec!["https://a.example".to_string()]));
        drop_pane("al-s-root");
    }
}
