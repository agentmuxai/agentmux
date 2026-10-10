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
//! (`browser_host_sync`), and stops navigations off it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use agentmux_common::allowed_origins;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::server::ui_handlers as ui;
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

/// `pane` was closed or taken over: it leaves its chain. The others keep the
/// list, even when `pane` was the root: a popup pane can outlive its opener
/// and still be the agent's, and must not lose its limit with it. The list
/// goes when its last pane does. Returns whether `pane` had a list.
pub(crate) fn drop_pane(pane: &str) -> bool {
    let mut l = lock();
    let Some(root) = l.members.remove(pane) else { return false };
    if !l.members.values().any(|r| *r == root) {
        l.lists.remove(&root);
    }
    drop(l);
    crate::server::browser_owner::changed().notify_one();
    true
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

/// Check `OpenBrowser`'s `allowed_origins`: `None` when not given, the list
/// in its normal form when it is, or why it can't be used.
pub(crate) fn list_for_open(entries: Option<&[String]>, url: &str) -> Result<Option<Vec<String>>, String> {
    let Some(entries) = entries else { return Ok(None) };
    let list = allowed_origins::parse_list(entries).map_err(|e| format!("allowed_origins: {e}"))?;
    if list.is_empty() {
        return Err("allowed_origins is empty: leave it out, or list the sites the pane may go to".to_string());
    }
    if !allowed_origins::allows(&list, url) {
        return Err(format!("url {url:?} isn't on allowed_origins ({})", list.join(", ")));
    }
    Ok(Some(list))
}

/// Mirror a new pane's list (if any) into the meta it's created with.
pub(crate) fn mirror(meta: &mut crate::backend::obj::MetaMapType, list: Option<&[String]>) {
    if let Some(list) = list {
        meta.insert(ALLOWED_META_KEY.to_string(), json!(list));
    }
}

/// Limit `pane`, which `OpenBrowser` just opened, to `list` if one was given,
/// and send it to the host at once, not on the next sync: the host must have
/// it before the pane's first navigation can leave it.
pub(crate) async fn limit_new_pane(state: &AppState, pane: &str, list: Option<Vec<String>>) {
    if let Some(list) = list {
        set(pane, list);
        crate::server::browser_host_sync::push(state).await;
    }
}

/// Why a popup pane to `url` from `opener` isn't opened, if it isn't: the
/// host asks before offering one off the opener's list, and this checks again
/// from srv's own record.
pub(crate) fn refuse_popup(opener: &str, url: &str) -> Option<&'static str> {
    list_for(opener)
        .is_some_and(|l| !allowed_origins::allows(&l, url))
        .then_some("it isn't on the sites the pane that opened it is limited to")
}

/// `pane`, a popup pane `opener` just opened, shares the site limit of the
/// chain it opened from, if that has one: in its header, and on the host
/// before its next navigation.
pub(crate) async fn join_popup_pane(state: &AppState, pane: &str, opener: &str) {
    if let Some(list) = join(pane, opener) {
        publish(state, &[pane.to_string()], Some(&list));
        crate::server::browser_host_sync::push(state).await;
    }
}

/// Why an agent's own `BrowserNavigate` of `pane` to `url` is refused, if it
/// is: the pane is limited to some sites and `url` isn't on them (§3).
pub(crate) fn refuse_agent_navigation(pane: &str, url: &str) -> Option<String> {
    let list = list_for(pane)?;
    (!allowed_origins::allows(&list, url)).then(|| {
        format!(
            "{url:?} isn't on the sites this pane is limited to ({}): open another pane with OpenBrowser \
             for it, or ask the user",
            list.join(", ")
        )
    })
}

/// `POST /api/v1/host/browser_navigation` — the host stopped a navigation off
/// a pane's site list (allowed-origins spec §4). Body `{pane, target, url,
/// kind}`: `pane` is the pane the list belongs to, `target` the pane or popup
/// window that was navigating, `kind` `navigate` (replayed on Allow) or
/// `popup` (not: the person or agent clicks again). Only the host calls it.
/// Answers at once; the question runs on its own.
pub(crate) async fn handle_host_browser_navigation(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use crate::server::{browser_allowlist, browser_attention};
    if !ui::from_host(&state, &headers).await {
        return ui::err_response(StatusCode::FORBIDDEN, "only the AgentMux host can report a navigation".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let (pane, target, url, kind) = (s("pane"), s("target"), s("url"), s("kind"));
    let answered = |asked: bool, reason: &str| {
        (StatusCode::OK, Json(json!({ "ok": true, "data": { "asked": asked, "reason": reason } }))).into_response()
    };
    let Some(list) = browser_allowlist::list_for(&pane) else {
        // Taken over or closed since: the host's copy was behind. Send it
        // the current one, and let the navigation it stopped go on.
        crate::server::browser_host_sync::push(&state).await;
        if kind == "navigate" {
            replay_navigation(&state, &target, &url).await;
        }
        return answered(false, "the pane isn't limited to any sites");
    };
    if allowed_origins::allows(&list, &url) {
        // Allowed already (the host's copy was behind): send it the list, and go on.
        crate::server::browser_host_sync::push(&state).await;
        if kind == "navigate" {
            replay_navigation(&state, &target, &url).await;
        }
        return answered(false, "it is on the pane's sites");
    }
    let Some(origin) = allowed_origins::target_origin(&url) else {
        return answered(false, "not a web address");
    };
    if browser_attention::waiting_on_user(&pane).is_some() {
        return answered(false, "the pane is already waiting for the user");
    }
    let agent = crate::server::browser_owner::owner_of(&pane).unwrap_or_default();
    let window = crate::server::browser_popup::is_window_id(&target)
        .then(|| crate::server::browser_popup::window(&target).map(|w| w.url))
        .flatten();
    tracing::info!(pane = %pane, target = %target, origin = %origin, kind = %kind, "[browser-allowlist] asking the user");
    let st = state.clone();
    tokio::spawn(async move {
        let asked = browser_attention::ask(
            &st,
            &pane,
            &agent,
            browser_attention::Kind::Navigation,
            json!({ "url": url, "origin": origin, "window": window, "popup": kind == "popup" }),
            std::time::Duration::from_secs(10 * 60),
        )
        .await;
        let (answer, waiting) = match asked {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!(pane = %pane, error = %e, "[browser-allowlist] couldn't ask");
                return;
            }
        };
        if answer != browser_attention::Answer::Yes {
            tracing::info!(pane = %pane, origin = %origin, answer = ?answer, "[browser-allowlist] not allowed");
        } else if let Some((list, members)) = browser_allowlist::allow(&pane, &origin) {
            // (None: taken over or closed meanwhile, no list left to add to.)
            tracing::info!(pane = %pane, origin = %origin, "[browser-allowlist] allowed by the user");
            browser_allowlist::publish(&st, &members, Some(&list));
            crate::server::browser_host_sync::push(&st).await;
            if kind == "navigate" {
                replay_navigation(&st, &target, &url).await;
            }
        }
        // The pane stays locked until the load is on its way. Then the host
        // hears the question is over, and reports this pane's attempts again.
        drop(waiting);
        crate::server::browser_host_sync::push(&st).await;
    });
    answered(true, "")
}

/// Load `url` in `target` (a pane or popup window), as the navigation the
/// host stopped would have. A plain load: a form's data isn't sent again.
async fn replay_navigation(state: &AppState, target: &str, url: &str) {
    let Ok(host) = ui::get_host_ipc(state).await else { return };
    if ui::proxy_ack(state, &host, "navigate", json!({ "block_id": target, "url": url })).await.is_err() {
        tracing::warn!(target = %target, "[browser-allowlist] couldn't load the allowed address");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_shares_one_list_and_keeps_it_past_its_root() {
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
        assert!(drop_pane("al-popup"));
        assert!(list_for("al-popup").is_none() && list_for("al-popup2").is_some());
        // The root leaving too: a popup pane that outlives it stays limited.
        assert!(drop_pane("al-root"));
        assert!(list_for("al-root").is_none());
        assert_eq!(list_for("al-popup2").unwrap().len(), 2);
        // A popup of that popup still joins the same list.
        assert_eq!(join("al-popup3", "al-popup2").unwrap().len(), 2);
        // The list goes with its last pane.
        assert!(drop_pane("al-popup2") && drop_pane("al-popup3"));
        assert!(!snapshot().keys().any(|k| k.starts_with("al-")));
        assert!(!lock().lists.contains_key("al-root"));
    }

    #[test]
    fn a_pane_without_a_list_has_nothing_to_join_or_allow() {
        assert!(join("al-x-popup", "al-x-unlisted").is_none());
        assert!(allow("al-x-unlisted", "https://b.example").is_none());
        assert!(!drop_pane("al-x-unlisted"));
    }

    #[test]
    fn the_snapshot_lists_every_member() {
        set("al-s-root", vec!["https://a.example".into()]);
        join("al-s-popup", "al-s-root");
        let snap = snapshot();
        assert_eq!(snap.get("al-s-root"), Some(&vec!["https://a.example".to_string()]));
        assert_eq!(snap.get("al-s-popup"), Some(&vec!["https://a.example".to_string()]));
        drop_pane("al-s-root");
        drop_pane("al-s-popup");
    }
}
