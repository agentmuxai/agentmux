// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which cookie jar a browser tab browses as: its block's `browser:identity`
//! (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §5–§6). Absent
//! for Personal; `incognito:<jar>` for an in-memory jar of its own, which the
//! CEF host makes on first use. Set when the tab is opened and never changed:
//! to browse as someone else, open another tab.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::backend::obj::{Block, MetaMapType};
use crate::server::AppState;

pub(crate) const IDENTITY_META_KEY: &str = "browser:identity";

/// Most Incognito jars open at once: each is its own Chrome profile with its
/// own renderer (identities spec §4.3). The host keeps the same cap.
pub(crate) const MAX_INCOGNITO_JARS: usize = 8;

/// Is `identity` a well-formed Incognito identity?
pub(crate) fn is_incognito(identity: &str) -> bool {
    identity
        .strip_prefix("incognito:")
        .is_some_and(|jar| (8..=64).contains(&jar.len()) && jar.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
}

fn identity_of(meta: &MetaMapType) -> Option<&str> {
    meta.get(IDENTITY_META_KEY).and_then(Value::as_str)
}

/// The Incognito identities blocks use now: the jars the host must keep.
/// `None` when the store can't be read: the host then keeps every jar, rather
/// than taking an empty list as "no tab uses any" and signing them all out.
pub(crate) fn live_jars(state: &AppState) -> Option<Vec<String>> {
    let blocks = match state.mstore.get_all::<Block>() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[browser-identity] couldn't list blocks; the host keeps its jars");
            return None;
        }
    };
    let set: HashSet<String> = blocks
        .iter()
        .filter_map(|b| identity_of(&b.meta))
        .filter(|i| is_incognito(i))
        .map(str::to_string)
        .collect();
    Some(set.into_iter().collect())
}

/// Check the identity a client gives a new browser tab (`pane.open`): an
/// Incognito one that is well formed, on a host that can give it a jar of its
/// own, within the cap unless it shares a jar already open. Anything else
/// fails the open rather than quietly browsing in the shared jar.
pub(crate) fn check_new_tab(state: &AppState, meta: &MetaMapType) -> Result<(), String> {
    let Some(v) = meta.get(IDENTITY_META_KEY) else { return Ok(()) };
    let identity = v.as_str().filter(|i| is_incognito(i)).ok_or_else(|| {
        if v.as_str().is_some_and(|i| i.starts_with("profile:")) {
            "named browser profiles aren't available yet".to_string()
        } else {
            format!("{IDENTITY_META_KEY} must be incognito:<id>")
        }
    })?;
    if !cfg!(windows) {
        return Err("Incognito tabs are Windows only for now".to_string());
    }
    let open = live_jars(state).ok_or("couldn't count the Incognito tabs open")?;
    if !open.iter().any(|j| j == identity) && open.len() >= MAX_INCOGNITO_JARS {
        return Err(format!(
            "at most {MAX_INCOGNITO_JARS} Incognito tabs can be open at once: close one to open another"
        ));
    }
    Ok(())
}

/// A popup pane browses as the tab that opened it: same jar, so a sign-in in
/// it signs in the tab.
pub(crate) fn inherit(meta: &mut MetaMapType, opener: &Block) {
    if let Some(identity) = identity_of(&opener.meta).filter(|i| is_incognito(i)) {
        meta.insert(IDENTITY_META_KEY.to_string(), json!(identity));
    }
}

/// How many more Incognito jars a layout replay may open: the cap less those
/// open now (none when they can't be counted, or off Windows).
pub(crate) fn jars_available(state: &AppState) -> usize {
    if !cfg!(windows) {
        return 0;
    }
    live_jars(state).map_or(0, |open| MAX_INCOGNITO_JARS.saturating_sub(open.len()))
}

/// For a layout being replayed: each Incognito identity becomes a fresh one,
/// so the replay doesn't share a jar with a tab still open. Tabs that shared
/// a jar keep sharing one. `fresh` maps old to new across the replay; at most
/// `available` fresh jars are made. Returns false for a tab that would need
/// one more: it is left out of the replay, not opened as a blank pane or in
/// the shared jar.
pub(crate) fn refresh_for_replay(meta: &mut Value, fresh: &mut HashMap<String, String>, available: usize) -> bool {
    let Some(obj) = meta.as_object_mut() else { return true };
    let Some(old) = obj.get(IDENTITY_META_KEY).and_then(Value::as_str).filter(|i| is_incognito(i)).map(str::to_string) else {
        return true;
    };
    if !fresh.contains_key(&old) && fresh.len() >= available {
        tracing::info!("[browser-identity] a replayed Incognito tab left out: no jar to spare");
        return false;
    }
    let new = fresh.entry(old).or_insert_with(|| format!("incognito:{}", uuid::Uuid::new_v4())).clone();
    obj.insert(IDENTITY_META_KEY.to_string(), json!(new));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_well_formed_incognito_identities_pass() {
        assert!(is_incognito("incognito:0f0e2d1c-aaaa-bbbb-cccc-0123456789ab"));
        for bad in ["incognito:short", "incognito:has spaces x", "profile:work", "incognito:", ""] {
            assert!(!is_incognito(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_replayed_layout_gets_fresh_jars_and_keeps_shared_ones_shared() {
        let mut fresh = HashMap::new();
        let mut a = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:aaaaaaaa-1111" });
        let mut b = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:aaaaaaaa-1111" });
        let mut c = json!({ "view": "browser" });
        assert!(refresh_for_replay(&mut a, &mut fresh, 8));
        assert!(refresh_for_replay(&mut b, &mut fresh, 8));
        assert!(refresh_for_replay(&mut c, &mut fresh, 8));
        let ia = a[IDENTITY_META_KEY].as_str().unwrap();
        assert_ne!(ia, "incognito:aaaaaaaa-1111");
        assert!(is_incognito(ia));
        assert_eq!(a[IDENTITY_META_KEY], b[IDENTITY_META_KEY]);
        assert!(c.get(IDENTITY_META_KEY).is_none());
        // Past the jars there are to spare, a tab needing a new one is left out;
        // one sharing a jar already made still comes.
        let mut d = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:dddddddd-4444" });
        assert!(!refresh_for_replay(&mut d, &mut fresh, 1));
        let mut e = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:aaaaaaaa-1111" });
        assert!(refresh_for_replay(&mut e, &mut fresh, 1));
    }

    #[test]
    fn a_popup_pane_inherits_an_incognito_jar_only() {
        let mut opener = Block::default();
        opener.meta.insert(IDENTITY_META_KEY.into(), json!("incognito:bbbbbbbb-2222"));
        let mut meta = MetaMapType::new();
        inherit(&mut meta, &opener);
        assert_eq!(meta[IDENTITY_META_KEY], json!("incognito:bbbbbbbb-2222"));
        let mut meta = MetaMapType::new();
        inherit(&mut meta, &Block::default());
        assert!(meta.get(IDENTITY_META_KEY).is_none());
    }

    #[tokio::test]
    async fn a_new_tab_identity_is_checked() {
        let state = crate::server::tests::test_state();
        let with = |v: Value| {
            let mut m = MetaMapType::new();
            m.insert(IDENTITY_META_KEY.into(), v);
            m
        };
        assert!(check_new_tab(&state, &MetaMapType::new()).is_ok());
        assert!(check_new_tab(&state, &with(json!("incognito:short"))).is_err());
        assert!(check_new_tab(&state, &with(json!("profile:work"))).unwrap_err().contains("aren't available"));
        assert!(check_new_tab(&state, &with(json!(5))).is_err());
        let ok = check_new_tab(&state, &with(json!("incognito:cccccccc-3333")));
        assert_eq!(ok.is_ok(), cfg!(windows), "{ok:?}");
    }
}
