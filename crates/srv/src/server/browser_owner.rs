// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which agent owns which agent-opened browser pane
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §3).
//!
//! An agent may drive a browser pane only if it opened that pane with
//! `OpenBrowser`. The authority is this in-memory map, written only by
//! `ui_handlers::handle_ui_browser_open` after verifying the caller's
//! signed identity. The block's `browser:owner_agent` meta key mirrors it
//! for the "Driven by" badge, and both must agree:
//!
//! - a client that writes the meta key itself gains nothing, since the map
//!   has no entry for it;
//! - a duplicated pane copies the meta key but gets a new block id, which
//!   the map doesn't know;
//! - the human's Take over clears the meta key; the client-write guard
//!   (`guard_client_meta_write`) then drops the map entry, so ownership ends
//!   for good, and no client can set the key at all;
//! - deleting the block drops its entry (`wcore::delete_block`);
//! - after srv restarts the map is empty, so agents reopen their panes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::obj::{Block, MetaMapType};

/// Block meta key mirroring the owner, for the frontend.
pub(crate) const OWNER_META_KEY: &str = "browser:owner_agent";

fn owners() -> &'static Mutex<HashMap<String, String>> {
    static OWNERS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OWNERS.get_or_init(Default::default)
}

/// Record `agent_id` as the owner of `block_id`. Only `OpenBrowser` calls this.
pub(crate) fn record(block_id: &str, agent_id: &str) {
    owners()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(block_id.to_string(), agent_id.to_string());
    changed().notify_one();
}

/// Drop `block_id`'s owner: the pane was deleted or taken over.
pub(crate) fn forget(block_id: &str) {
    let removed = owners()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(block_id)
        .is_some();
    if removed {
        changed().notify_one();
    }
}

/// Every pane an agent owns. The host keeps a copy, to decide inside
/// `on_before_popup` which of a pane's popups open in-app
/// (SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §3); it is
/// only a hint there, since who may drive what is always checked here.
pub(crate) fn owned_panes() -> Vec<String> {
    let mut v: Vec<String> = owners()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .keys()
        .cloned()
        .collect();
    v.sort();
    v
}

/// Woken whenever the set of owned panes changes, or the host needs the
/// whole set again (it registered). See `ui_handlers::spawn_owned_panes_sync`.
pub(crate) fn changed() -> &'static tokio::sync::Notify {
    static CHANGED: OnceLock<tokio::sync::Notify> = OnceLock::new();
    CHANGED.get_or_init(tokio::sync::Notify::new)
}

pub(crate) fn owner_of(block_id: &str) -> Option<String> {
    owners()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(block_id)
        .cloned()
}

/// Why a caller may not drive a pane. Not an authentication failure: the
/// caller's identity was already verified.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Denied {
    /// No such block (closed, or never existed).
    NotFound(String),
    /// The block exists, but this caller may not drive it.
    Forbidden(String),
}

impl Denied {
    pub(crate) fn message(&self) -> &str {
        match self {
            Denied::NotFound(m) | Denied::Forbidden(m) => m,
        }
    }
}

/// May `agent_id` drive `pane`? `block` is the pane's block as stored (None if
/// it no longer exists) and `recorded` its entry in the owner map.
pub(crate) fn check(
    block: Option<&Block>,
    recorded: Option<&str>,
    agent_id: &str,
    pane: &str,
) -> Result<(), Denied> {
    let block = block
        .ok_or_else(|| Denied::NotFound(format!("no pane {pane:?} (it may have been closed)")))?;
    let view = block
        .meta
        .get("view")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if view != "browser" {
        return Err(Denied::Forbidden(format!(
            "pane {pane:?} is not a browser pane"
        )));
    }
    if !recorded.is_some_and(|r| r.eq_ignore_ascii_case(agent_id)) {
        return Err(Denied::Forbidden(format!(
            "pane {pane:?} is not a browser pane you own: an agent can drive only browser \
             panes it opened itself with OpenBrowser, and loses one the user takes over"
        )));
    }
    let mirrored = block
        .meta
        .get(OWNER_META_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !mirrored.eq_ignore_ascii_case(agent_id) {
        return Err(Denied::Forbidden(format!(
            "the user took over pane {pane:?}; open a new one with OpenBrowser if you still need a browser"
        )));
    }
    Ok(())
}

/// Guard for meta updates that come from a client (the `setmeta` WebSocket
/// command and the `UpdateObjectMeta` service call), not from srv itself.
/// `browser:owner_agent` is srv's to write: a client may only clear it, which
/// is the human's Take over and ends that pane's ownership for good. Setting
/// or changing it is refused, so a taken-over agent can't write itself back.
/// `browser:attention` (the hand-off and approval banner) is srv's alone:
/// a client that could write it could keep a real request's id and change
/// what the banner tells the human they're approving.
pub(crate) fn guard_client_meta_write(oref: &str, meta: &MetaMapType) -> Result<(), String> {
    // The popup keys name the pane that opened this one and its site, shown
    // to the person as "Popup from …": a client that could write them could
    // make any pane claim to come from a site it doesn't.
    for key in [
        crate::server::browser_attention::ATTENTION_META_KEY,
        crate::server::browser_popup::POPUP_OF_META_KEY,
        crate::server::browser_popup::POPUP_FROM_META_KEY,
        crate::server::browser_popup::POPUP_WINDOWS_META_KEY,
    ] {
        if meta.contains_key(key) {
            return Err(format!("{key} is written only by AgentMux itself"));
        }
    }
    let Some(value) = meta.get(OWNER_META_KEY) else {
        return Ok(());
    };
    if !value.is_null() {
        return Err(format!(
            "{OWNER_META_KEY} is written by AgentMux itself when an agent opens a browser pane; \
             a client can only clear it (Take over)"
        ));
    }
    if let Some(block_id) = oref.strip_prefix("block:") {
        forget(block_id);
        // Take over also answers any hand-off or approval still up on the
        // pane: an Approve clicked after it must not go through.
        crate::server::browser_attention::cancel_for(block_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(view: &str, owner: Option<&str>) -> Block {
        let mut b = Block::default();
        b.meta.insert("view".to_string(), json!(view));
        if let Some(o) = owner {
            b.meta.insert(OWNER_META_KEY.to_string(), json!(o));
        }
        b
    }

    #[test]
    fn the_opener_may_drive_its_pane() {
        let b = block("browser", Some("lark"));
        assert!(check(Some(&b), Some("lark"), "lark", "p1").is_ok());
        // Agent ids compare case-insensitively, as elsewhere.
        assert!(check(Some(&b), Some("Lark"), "lark", "p1").is_ok());
    }

    #[test]
    fn another_agent_may_not() {
        let b = block("browser", Some("lark"));
        let e = check(Some(&b), Some("lark"), "korp", "p1").unwrap_err();
        assert!(
            matches!(&e, Denied::Forbidden(m) if m.contains("not a browser pane you own")),
            "{e:?}"
        );
    }

    #[test]
    fn a_forged_meta_key_without_a_recorded_owner_is_refused() {
        // A client wrote the meta key, or the pane was duplicated: no map entry.
        let b = block("browser", Some("lark"));
        assert!(check(Some(&b), None, "lark", "p1").is_err());
    }

    #[test]
    fn take_over_clears_the_meta_key_and_ends_ownership() {
        let b = block("browser", None);
        let e = check(Some(&b), Some("lark"), "lark", "p1").unwrap_err();
        assert!(
            matches!(&e, Denied::Forbidden(m) if m.contains("took over")),
            "{e:?}"
        );
    }

    #[test]
    fn only_browser_panes_and_only_existing_ones() {
        let b = block("term", Some("lark"));
        assert!(matches!(
            check(Some(&b), Some("lark"), "lark", "p1"),
            Err(Denied::Forbidden(_))
        ));
        assert!(matches!(
            check(None, Some("lark"), "lark", "p1"),
            Err(Denied::NotFound(_))
        ));
    }

    #[test]
    fn record_owner_of_and_forget() {
        record("test-block-browser-owner-rt", "lark");
        assert_eq!(
            owner_of("test-block-browser-owner-rt").as_deref(),
            Some("lark")
        );
        assert_eq!(owner_of("test-block-browser-owner-unknown"), None);
        forget("test-block-browser-owner-rt");
        assert_eq!(owner_of("test-block-browser-owner-rt"), None);
    }

    fn meta(key: &str, v: serde_json::Value) -> MetaMapType {
        let mut m = MetaMapType::new();
        m.insert(key.to_string(), v);
        m
    }

    #[test]
    fn a_client_cannot_set_the_owner_key() {
        // The taken-over agent writing itself back, or any client faking an owner.
        let e =
            guard_client_meta_write("block:test-guard-set", &meta(OWNER_META_KEY, json!("lark")))
                .unwrap_err();
        assert!(e.contains("only clear it"), "{e}");
    }

    #[test]
    fn a_client_clearing_the_owner_key_ends_ownership_for_good() {
        record("test-guard-clear", "lark");
        guard_client_meta_write(
            "block:test-guard-clear",
            &meta(OWNER_META_KEY, serde_json::Value::Null),
        )
        .unwrap();
        assert_eq!(owner_of("test-guard-clear"), None);
        // Even if the key were somehow written back, the map no longer agrees.
        let b = block("browser", Some("lark"));
        assert!(check(
            Some(&b),
            owner_of("test-guard-clear").as_deref(),
            "lark",
            "test-guard-clear"
        )
        .is_err());
    }

    #[test]
    fn a_client_cannot_write_the_banner_at_all() {
        // Rewriting a real request's banner (same id, harmless-looking text)
        // would make the human approve something they weren't shown.
        let key = crate::server::browser_attention::ATTENTION_META_KEY;
        let forged = json!({ "id": "real-id", "kind": "approval", "what": "Click \"Cancel\"" });
        assert!(guard_client_meta_write("block:test-guard-attn", &meta(key, forged)).is_err());
        assert!(guard_client_meta_write("block:test-guard-attn", &meta(key, serde_json::Value::Null)).is_err());
    }

    #[test]
    fn a_client_cannot_write_the_popup_keys() {
        use crate::server::browser_popup::{POPUP_FROM_META_KEY, POPUP_OF_META_KEY, POPUP_WINDOWS_META_KEY};
        for key in [POPUP_OF_META_KEY, POPUP_FROM_META_KEY, POPUP_WINDOWS_META_KEY] {
            assert!(guard_client_meta_write("block:test-guard-popup", &meta(key, json!("x"))).is_err());
            assert!(guard_client_meta_write("block:test-guard-popup", &meta(key, serde_json::Value::Null)).is_err());
        }
    }

    #[test]
    fn other_meta_writes_pass_untouched() {
        record("test-guard-other", "lark");
        guard_client_meta_write(
            "block:test-guard-other",
            &meta("url", json!("https://example.com")),
        )
        .unwrap();
        assert_eq!(owner_of("test-guard-other").as_deref(), Some("lark"));
        forget("test-guard-other");
    }
}
