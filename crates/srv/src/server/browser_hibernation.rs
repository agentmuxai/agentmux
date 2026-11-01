// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's `Browser` call on a hibernated browser pane wakes it first
//! (docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §8).
//!
//! A hibernated pane has no page: its window closed it to free memory and keeps
//! `browser:sleep` in the block's meta until a wake has loaded it again. srv
//! can't create the page itself (the window owns where it goes), so it asks:
//! it writes `browser:wake`, which the pane's view answers by waking, and waits
//! for `browser:sleep` to clear.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::server::AppState;
use crate::backend::obj::{Block, MetaMapType};

/// Block meta while a browser pane is asleep or waking.
pub(crate) const SLEEP_META_KEY: &str = "browser:sleep";
/// Block meta srv writes to ask the pane's view to wake it.
pub(crate) const WAKE_META_KEY: &str = "browser:wake";

/// Block meta srv writes when an agent's `Browser` call reaches the pane (ms). The
/// action log holds only what changed the page, so a pane an agent reads (snapshot,
/// find) would look unused; the window counts this too before letting it sleep.
pub(crate) const LAST_CALL_META_KEY: &str = "browser:lastcall";
/// A call is noted at most this often per pane: the window's sleep time is minutes.
const LAST_CALL_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long a `Browser` call waits for a pane to wake before failing.
const WAKE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(150);

/// Whether a call at `now` is due to be noted, given when the last one was.
fn call_due(last: Option<Instant>, now: Instant) -> bool {
    last.map_or(true, |t| now.saturating_duration_since(t) >= LAST_CALL_EVERY)
}

/// Notes in the pane's meta that an agent's call reached it, at most once per
/// `LAST_CALL_EVERY`. A failed write is only a missed note.
fn note_call(state: &AppState, block_id: &str) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let now = Instant::now();
    {
        let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        if !call_due(last.get(block_id).copied(), now) {
            return;
        }
        last.insert(block_id.to_string(), now);
    }
    let mut meta = MetaMapType::new();
    meta.insert(LAST_CALL_META_KEY.to_string(), json!(chrono::Utc::now().timestamp_millis()));
    let _ = super::http_shell::broadcast_meta_update(state, block_id, &meta);
}

fn is_asleep(block: &Block) -> bool {
    block
        .meta
        .get(SLEEP_META_KEY)
        .is_some_and(|v| v.is_object())
}

fn asleep(state: &AppState, block_id: &str) -> bool {
    state
        .mstore
        .get::<Block>(block_id)
        .ok()
        .flatten()
        .is_some_and(|b| is_asleep(&b))
}

/// Wakes `block_id` if it is a hibernated browser pane, and waits for its page.
/// `Ok(true)` when it was asleep and is awake now, `Ok(false)` when it wasn't
/// asleep (or isn't a browser pane).
pub(crate) async fn ensure_awake(state: &AppState, block_id: &str) -> Result<bool, String> {
    if block_id.is_empty() || !asleep(state, block_id) {
        return Ok(false);
    }
    tracing::info!(block_id = %block_id, "[hibernate] waking a sleeping browser pane for an agent's call");
    let mut meta = MetaMapType::new();
    meta.insert(WAKE_META_KEY.to_string(), json!(chrono::Utc::now().timestamp_millis()));
    super::http_shell::broadcast_meta_update(state, block_id, &meta)?;
    let end = Instant::now() + WAKE_TIMEOUT;
    while Instant::now() < end {
        tokio::time::sleep(POLL).await;
        if !asleep(state, block_id) {
            return Ok(true);
        }
    }
    Err(format!(
        "this browser pane was asleep to save memory and didn't wake within {} s; \
         is its window open? Try again, or open the pane",
        WAKE_TIMEOUT.as_secs()
    ))
}

/// Every srv call into the host's browser API goes through here: a hibernated
/// browser pane named by the body's `block_id` has no page, so it is woken
/// first, and the answer says so.
pub(crate) async fn post_to_host_awake(
    state: &AppState,
    host: &crate::server::HostIpc,
    route: &str,
    body: serde_json::Value,
    timeout: Option<Duration>,
) -> Result<serde_json::Value, String> {
    let block_id = body.get("block_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let woke = ensure_awake(state, &block_id).await?;
    if !block_id.is_empty() {
        note_call(state, &block_id);
    }
    let resp = super::host_http::post_to_host(&state.http_client, host, route, body, timeout).await?;
    Ok(if woke { mark_woken(resp) } else { resp })
}

/// The host's answer to a call that woke the pane, marked so the agent's tool
/// can say its page was reloaded.
pub(crate) fn mark_woken(mut host_resp: serde_json::Value) -> serde_json::Value {
    if let Some(data) = host_resp.get_mut("data").and_then(|d| d.as_object_mut()) {
        data.insert("woke_from_sleep".to_string(), json!(true));
    }
    host_resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sleep_record_is_an_object_and_null_is_awake() {
        let mut b = Block::default();
        assert!(!is_asleep(&b));
        b.meta.insert(SLEEP_META_KEY.into(), serde_json::Value::Null);
        assert!(!is_asleep(&b));
        b.meta.insert(SLEEP_META_KEY.into(), json!({ "since": 1, "url": "https://x" }));
        assert!(is_asleep(&b));
    }

    #[test]
    fn a_pane_an_agent_owns_is_woken_by_the_agents_call_like_any_other() {
        let mut b = Block::default();
        b.meta.insert("browser:owner_agent".into(), json!("agent-x"));
        b.meta.insert(SLEEP_META_KEY.into(), json!({ "since": 1, "reason": "hidden", "url": "https://x" }));
        assert!(is_asleep(&b), "the sleep record is all `ensure_awake` looks at: ownership never blocks the wake");
    }

    #[test]
    fn a_call_is_noted_once_in_a_while_not_on_every_snapshot() {
        let t0 = Instant::now();
        assert!(call_due(None, t0));
        assert!(!call_due(Some(t0), t0 + Duration::from_secs(60)));
        assert!(!call_due(Some(t0), t0 + LAST_CALL_EVERY - Duration::from_secs(1)));
        assert!(call_due(Some(t0), t0 + LAST_CALL_EVERY));
    }

    #[test]
    fn a_woken_answer_says_so_in_its_data() {
        let r = mark_woken(json!({ "ok": true, "data": { "snapshot": "x" } }));
        assert_eq!(r["data"]["woke_from_sleep"], json!(true));
        let r = mark_woken(json!({ "ok": false, "error": "e" }));
        assert!(r.get("data").is_none());
    }
}
