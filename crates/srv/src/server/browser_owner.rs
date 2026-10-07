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
//! - the human's Take over clears the meta key, which ends ownership at once;
//! - after srv restarts the map is empty, so agents reopen their panes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::obj::Block;

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
}

pub(crate) fn owner_of(block_id: &str) -> Option<String> {
    owners()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(block_id)
        .cloned()
}

/// May `agent_id` drive `pane`? `block` is the pane's block as stored (None if
/// it no longer exists) and `recorded` its entry in the owner map.
pub(crate) fn check(
    block: Option<&Block>,
    recorded: Option<&str>,
    agent_id: &str,
    pane: &str,
) -> Result<(), String> {
    let block = block.ok_or_else(|| format!("no pane {pane:?} (it may have been closed)"))?;
    let view = block.meta.get("view").and_then(|v| v.as_str()).unwrap_or("");
    if view != "browser" {
        return Err(format!("pane {pane:?} is not a browser pane"));
    }
    if !recorded.is_some_and(|r| r.eq_ignore_ascii_case(agent_id)) {
        return Err(format!(
            "pane {pane:?} is not a browser pane you opened with OpenBrowser; \
             an agent can drive only browser panes it opened itself"
        ));
    }
    let mirrored = block.meta.get(OWNER_META_KEY).and_then(|v| v.as_str()).unwrap_or("");
    if !mirrored.eq_ignore_ascii_case(agent_id) {
        return Err(format!(
            "the user took over pane {pane:?}; open a new one with OpenBrowser if you still need a browser"
        ));
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
        assert!(e.contains("not a browser pane you opened"), "{e}");
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
        assert!(e.contains("took over"), "{e}");
    }

    #[test]
    fn only_browser_panes_and_only_existing_ones() {
        let b = block("term", Some("lark"));
        assert!(check(Some(&b), Some("lark"), "lark", "p1").unwrap_err().contains("not a browser pane"));
        assert!(check(None, Some("lark"), "lark", "p1").unwrap_err().contains("no pane"));
    }

    #[test]
    fn record_and_owner_of_round_trip() {
        record("test-block-browser-owner-rt", "lark");
        assert_eq!(owner_of("test-block-browser-owner-rt").as_deref(), Some("lark"));
        assert_eq!(owner_of("test-block-browser-owner-unknown"), None);
    }
}
