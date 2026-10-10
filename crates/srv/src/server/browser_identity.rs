// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which cookie jar a browser tab browses as: its block's `browser:identity`
//! (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §5–§6). Absent
//! for Personal; `incognito:<jar>` for an in-memory jar of its own, which the
//! CEF host makes on first use. Set when the tab is opened and never changed:
//! to browse as someone else, open another tab.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::backend::browser_profiles_store::BrowserProfile;
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

/// The profile id of a `profile:<id>` identity.
pub(crate) fn profile_id(identity: &str) -> Option<&str> {
    identity
        .strip_prefix("profile:")
        .filter(|id| crate::backend::browser_profiles_store::is_profile_id(id))
}

/// The ids of the named profiles there are, or `None` when they can't be read.
pub(crate) fn profile_ids() -> Option<HashSet<String>> {
    match crate::server::app_api::browser_profiles_list() {
        Ok(ps) => Some(ps.into_iter().map(|p| p.id).collect()),
        Err(e) => {
            tracing::warn!(error = %e, "[browser-identity] couldn't read the browser profiles");
            None
        }
    }
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
    let identity = v.as_str().unwrap_or("");
    if let Some(id) = profile_id(identity) {
        if !cfg!(windows) {
            return Err("browser profiles are Windows only for now".to_string());
        }
        let known = profile_ids().ok_or("couldn't read the browser profiles")?;
        return if known.contains(id) { Ok(()) } else { Err(format!("there is no browser profile {id:?}")) };
    }
    if !is_incognito(identity) {
        return Err(format!("{IDENTITY_META_KEY} must be incognito:<id> or profile:<id>"));
    }
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
    if let Some(identity) = identity_of(&opener.meta).filter(|i| is_incognito(i) || profile_id(i).is_some()) {
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
pub(crate) fn refresh_for_replay(
    meta: &mut Value,
    fresh: &mut HashMap<String, String>,
    available: usize,
    profiles: Option<&HashSet<String>>,
) -> bool {
    let Some(obj) = meta.as_object_mut() else { return true };
    // A tab of a profile that no longer exists is left out, not reopened in
    // a jar the host would make anew under the old id.
    if let Some(id) = obj.get(IDENTITY_META_KEY).and_then(Value::as_str).and_then(profile_id) {
        return profiles.is_some_and(|known| known.contains(id));
    }
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

/// The identity an agent's `OpenBrowser` asked for by `profile` browses as:
/// none (Personal) for "personal" or nothing; a fresh Incognito jar for
/// "incognito", always allowed since it gives the agent less, not more; and a
/// named profile (by name, any case, or id) only when the user switched on
/// "Agents may use this profile" for it (profiles spec §5).
pub(crate) fn identity_for_agent(requested: &str, profiles: &[BrowserProfile]) -> Result<Option<String>, String> {
    let requested = requested.trim();
    if requested.is_empty() || requested.eq_ignore_ascii_case("personal") {
        return Ok(None);
    }
    if requested.eq_ignore_ascii_case("incognito") {
        return Ok(Some(format!("incognito:{}", uuid::Uuid::new_v4())));
    }
    let allowed = || {
        let names: Vec<&str> = profiles.iter().filter(|p| p.agents_allowed).map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        }
    };
    let Some(p) = profiles.iter().find(|p| p.name.eq_ignore_ascii_case(requested) || p.id == requested) else {
        return Err(format!(
            "there is no browser profile called {requested:?}. Use \"incognito\", or a profile the user lets agents use: {}",
            allowed()
        ));
    };
    if !p.agents_allowed {
        return Err(format!(
            "the user hasn't let agents use the browser profile {:?}: they can switch on \"Agents may use this profile\"              in Settings → Browser. Use \"incognito\", or a profile agents may use: {}",
            p.name,
            allowed()
        ));
    }
    Ok(Some(format!("profile:{}", p.id)))
}

/// `OpenBrowser`'s `profile`: give the agent's new tab the identity it asked
/// for, checked like any new tab (the platform, the Incognito cap).
pub(crate) fn for_agent_open(state: &AppState, meta: &mut MetaMapType, requested: Option<&str>) -> Result<(), String> {
    let Some(requested) = requested else { return Ok(()) };
    let profiles = crate::server::app_api::browser_profiles_list()?;
    if let Some(identity) = identity_for_agent(requested, &profiles)? {
        meta.insert(IDENTITY_META_KEY.to_string(), json!(identity));
        check_new_tab(state, meta)?;
    }
    Ok(())
}

/// An agent opening a browser tab some other way (`pane.open`, the HTTP pane
/// open) with a `browser:identity`: a named profile only if agents may use it.
pub(crate) fn check_agent_may_use(meta: &MetaMapType) -> Result<(), String> {
    let Some(id) = identity_of(meta).and_then(profile_id) else { return Ok(()) };
    let profiles = crate::server::app_api::browser_profiles_list()?;
    match profiles.iter().find(|p| p.id == id) {
        Some(p) if p.agents_allowed => Ok(()),
        Some(p) => Err(format!("the user hasn't let agents use the browser profile {:?}", p.name)),
        None => Err(format!("there is no browser profile {id:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, name: &str, agents_allowed: bool) -> BrowserProfile {
        BrowserProfile { id: id.into(), name: name.into(), color: "#3b82f6".into(), created_at: 0, agents_allowed }
    }

    #[test]
    fn an_agent_gets_personal_incognito_or_a_profile_it_is_allowed() {
        let ps = [profile("p-work", "Work", true), profile("p-bank", "Bank", false)];
        assert_eq!(identity_for_agent("", &ps), Ok(None));
        assert_eq!(identity_for_agent("Personal", &ps), Ok(None));
        let a = identity_for_agent("incognito", &ps).unwrap().unwrap();
        let b = identity_for_agent("Incognito", &ps).unwrap().unwrap();
        assert!(is_incognito(&a) && is_incognito(&b) && a != b, "a fresh jar each time");
        assert_eq!(identity_for_agent("work", &ps), Ok(Some("profile:p-work".into())));
        assert_eq!(identity_for_agent("p-work", &ps), Ok(Some("profile:p-work".into())));
        let refused = identity_for_agent("Bank", &ps).unwrap_err();
        assert!(refused.contains("hasn't let agents use") && refused.contains("Work"), "{refused}");
        let unknown = identity_for_agent("Nope", &ps).unwrap_err();
        assert!(unknown.contains("no browser profile") && !unknown.contains("Bank"), "{unknown}");
    }

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
        assert!(refresh_for_replay(&mut a, &mut fresh, 8, None));
        assert!(refresh_for_replay(&mut b, &mut fresh, 8, None));
        assert!(refresh_for_replay(&mut c, &mut fresh, 8, None));
        let ia = a[IDENTITY_META_KEY].as_str().unwrap();
        assert_ne!(ia, "incognito:aaaaaaaa-1111");
        assert!(is_incognito(ia));
        assert_eq!(a[IDENTITY_META_KEY], b[IDENTITY_META_KEY]);
        assert!(c.get(IDENTITY_META_KEY).is_none());
        // Past the jars there are to spare, a tab needing a new one is left out;
        // one sharing a jar already made still comes.
        let mut d = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:dddddddd-4444" });
        assert!(!refresh_for_replay(&mut d, &mut fresh, 1, None));
        let mut e = json!({ "view": "browser", IDENTITY_META_KEY: "incognito:aaaaaaaa-1111" });
        assert!(refresh_for_replay(&mut e, &mut fresh, 1, None));
        // A profile tab comes only while its profile exists, and keeps its id.
        let known: HashSet<String> = ["p-work".to_string()].into();
        let mut f = json!({ "view": "browser", IDENTITY_META_KEY: "profile:p-work" });
        assert!(refresh_for_replay(&mut f, &mut fresh, 0, Some(&known)));
        assert_eq!(f[IDENTITY_META_KEY], json!("profile:p-work"));
        let mut g = json!({ "view": "browser", IDENTITY_META_KEY: "profile:p-gone" });
        assert!(!refresh_for_replay(&mut g, &mut fresh, 0, Some(&known)));
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
        let mut work = Block::default();
        work.meta.insert(IDENTITY_META_KEY.into(), json!("profile:p-work"));
        let mut meta = MetaMapType::new();
        inherit(&mut meta, &work);
        assert_eq!(meta[IDENTITY_META_KEY], json!("profile:p-work"));
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
        assert!(check_new_tab(&state, &with(json!("profile:p-none"))).is_err());
        assert!(check_new_tab(&state, &with(json!("profile:Bad Id"))).is_err());
        assert!(check_new_tab(&state, &with(json!(5))).is_err());
        let ok = check_new_tab(&state, &with(json!("incognito:cccccccc-3333")));
        assert_eq!(ok.is_ok(), cfg!(windows), "{ok:?}");
    }
}
