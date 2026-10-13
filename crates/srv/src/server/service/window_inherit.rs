// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handler — `InheritWindowAppearance`: a new window opens
//! with the theme and opacity of the window that created it
//! (docs/specs/SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12.md §6).
//!
//! The creator learns the new window's host label when `open_new_window` or
//! a tear-off returns, and calls this with its own window id and that label.
//! The new window's renderer creates its Window row (`CreateWindow`) at about
//! the same moment, so the two calls race. Each side first publishes what it
//! has, then checks for the other: this handler records a pending entry and
//! then looks for the row; `CreateWindow` records its row and then takes any
//! pending entry. Whichever side takes the entry applies it, so it is applied
//! exactly once whatever the order.
//!
//! Host labels are reused (a closed pool window is demoted and promoted
//! again), and the old row's `host:label` crumb can outlive the reuse: its
//! close runs on another thread. So this handler doesn't look rows up by
//! crumb. It only takes a row `CreateWindow` made for the label within
//! `JUST_CREATED` (a new window creates its row within moments of the
//! request that opened it); otherwise the entry waits for `CreateWindow`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

/// The per-window appearance keys a new window copies from its creator.
pub(crate) const INHERITED_KEYS: &[&str] = &["window:theme", "window:opacity"];

/// How long a pending entry waits for its window. A new window creates its
/// row within a second or two; host labels are reused (pool windows are
/// demoted and promoted again), so a stale entry must not reach a later
/// window that happens to get the same label.
const PENDING_TTL: Duration = Duration::from_secs(30);

/// How recently `CreateWindow` must have made a label's row for a request
/// to apply to it at once.
const JUST_CREATED: Duration = Duration::from_secs(5);

/// host label → (source window id, when recorded).
static PENDING: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// host label → (the window id `CreateWindow` last made for it, when).
static CREATED: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Called by `CreateWindow` once the row exists, before it takes a pending
/// entry.
pub(crate) fn record_created(label: &str, window_id: &str) {
    let mut created = CREATED.lock().unwrap_or_else(|e| e.into_inner());
    created.retain(|_, (_, at)| at.elapsed() < PENDING_TTL);
    created.insert(label.to_string(), (window_id.to_string(), Instant::now()));
}

/// The row `CreateWindow` made for `label` within `JUST_CREATED`, if any.
fn just_created(label: &str) -> Option<String> {
    let created = CREATED.lock().unwrap_or_else(|e| e.into_inner());
    created
        .get(label)
        .filter(|(_, at)| at.elapsed() < JUST_CREATED)
        .map(|(id, _)| id.clone())
}

fn record_pending(label: &str, source_window_id: &str) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    pending.retain(|_, (_, at)| at.elapsed() < PENDING_TTL);
    pending.insert(label.to_string(), (source_window_id.to_string(), Instant::now()));
}

/// Take the pending source for `label`, if one was recorded recently.
pub(crate) fn take_pending(label: &str) -> Option<String> {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let (source, at) = pending.remove(label)?;
    (at.elapsed() < PENDING_TTL).then_some(source)
}

/// The source window's appearance keys that are set, as a meta patch.
/// `None` when the source is gone or has none of them set.
pub(crate) fn appearance_patch(source: &Window) -> Option<serde_json::Value> {
    let patch: serde_json::Map<String, serde_json::Value> = INHERITED_KEYS
        .iter()
        .filter_map(|k| {
            source
                .meta
                .get(*k)
                .filter(|v| !v.is_null())
                .map(|v| (k.to_string(), v.clone()))
        })
        .collect();
    (!patch.is_empty()).then(|| serde_json::Value::Object(patch))
}

/// Copy `source_window_id`'s appearance onto `target_window_id`. Returns the
/// events to publish; empty when there is nothing to copy.
pub(crate) async fn apply_inheritance(
    state: &AppState,
    source_window_id: &str,
    target_window_id: &str,
) -> Vec<agentmux_common::ipc::Event> {
    let store = &state.mstore;
    let Some(patch) = store
        .get::<Window>(source_window_id)
        .ok()
        .flatten()
        .and_then(|w| appearance_patch(&w))
    else {
        return Vec::new();
    };
    let evs = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::UpdateWindowMeta {
            window_id: target_window_id.to_string(),
            meta_patch: patch,
        },
    )
    .await;
    for ev in &evs {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, store) {
            tracing::warn!(
                window_id = %target_window_id,
                error = %e,
                "InheritWindowAppearance: meta write failed — window keeps the defaults"
            );
        }
    }
    evs
}

/// Args: `sourceWindowId`, `hostLabel` (the new window's host label).
pub(crate) async fn handle_inherit_window_appearance(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args = &call.args;
    let source_window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let label: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if source_window_id.is_empty() || label.is_empty() {
        return WebReturnType::error("InheritWindowAppearance: empty sourceWindowId or hostLabel".to_string());
    }
    record_pending(&label, &source_window_id);
    // The row may already exist (its renderer won the race). If so, and
    // CreateWindow hasn't already taken the entry, apply it here.
    if let Some(target) = just_created(&label) {
        if target != source_window_id {
            if let Some(source) = take_pending(&label) {
                let evs = apply_inheritance(state, &source, &target).await;
                publish_events(state, &evs);
            }
        }
    }
    WebReturnType::success_empty()
}

#[cfg(test)]
mod tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn call(method: &str, args: Vec<serde_json::Value>) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: method.to_string(),
            uicontext: None,
            args,
        }
    }

    async fn create(state: &crate::server::AppState, label: &str) -> String {
        let ret = handle_window_service(
            state,
            &call("CreateWindow", vec![serde_json::Value::Null, "".into(), label.into()]),
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        ret.data
            .as_ref()
            .and_then(|d| d.get("oid"))
            .and_then(|v| v.as_str())
            .expect("CreateWindow returns the Window with oid")
            .to_string()
    }

    async fn set_meta(state: &crate::server::AppState, window_id: &str, patch: serde_json::Value) {
        let evs = super::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::UpdateWindowMeta {
                window_id: window_id.to_string(),
                meta_patch: patch,
            },
        )
        .await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
    }

    fn meta(state: &crate::server::AppState, window_id: &str, key: &str) -> Option<serde_json::Value> {
        state.mstore.must_get::<Window>(window_id).unwrap().meta.get(key).cloned()
    }

    async fn inherit(state: &crate::server::AppState, source: &str, label: &str) {
        let ret = handle_window_service(state, &call("InheritWindowAppearance", vec![source.into(), label.into()])).await;
        assert!(ret.success, "InheritWindowAppearance failed: {:?}", ret.error);
    }

    /// The creator's call lands first: CreateWindow applies it as it makes the row.
    #[tokio::test]
    async fn inherits_when_the_creator_calls_before_the_window_exists() {
        let state = test_state();
        let source = create(&state, "inherit-src-a").await;
        set_meta(&state, &source, serde_json::json!({ "window:theme": "dracula", "window:opacity": 0.7 })).await;

        inherit(&state, &source, "inherit-new-a").await;
        let new = create(&state, "inherit-new-a").await;

        assert_eq!(meta(&state, &new, "window:theme"), Some("dracula".into()));
        assert_eq!(meta(&state, &new, "window:opacity"), Some(serde_json::json!(0.7)));
    }

    /// The new window's row exists first: the creator's call applies it.
    #[tokio::test]
    async fn inherits_when_the_window_exists_before_the_creator_calls() {
        let state = test_state();
        let source = create(&state, "inherit-src-b").await;
        set_meta(&state, &source, serde_json::json!({ "window:theme": "nord" })).await;

        let new = create(&state, "inherit-new-b").await;
        assert_eq!(meta(&state, &new, "window:theme"), None);
        inherit(&state, &source, "inherit-new-b").await;

        assert_eq!(meta(&state, &new, "window:theme"), Some("nord".into()));
        // Keys the source doesn't set aren't written as nulls.
        assert_eq!(meta(&state, &new, "window:opacity"), None);
    }

    /// A window opened with no inheritance pending keeps the defaults, and an
    /// entry is used once: a later window with the same label gets nothing.
    #[tokio::test]
    async fn an_entry_is_used_once() {
        let state = test_state();
        let source = create(&state, "inherit-src-c").await;
        set_meta(&state, &source, serde_json::json!({ "window:theme": "dracula" })).await;

        let plain = create(&state, "inherit-new-c").await;
        assert_eq!(meta(&state, &plain, "window:theme"), None);

        inherit(&state, &source, "inherit-new-d").await;
        let first = create(&state, "inherit-new-d").await;
        let second = create(&state, "inherit-new-d").await;
        assert_eq!(meta(&state, &first, "window:theme"), Some("dracula".into()));
        assert_eq!(meta(&state, &second, "window:theme"), None);
    }

    /// A reused label whose old row is still around, but wasn't just created,
    /// doesn't take the entry: it waits for the new window's CreateWindow.
    #[tokio::test]
    async fn a_stale_row_with_a_reused_label_does_not_take_it() {
        let state = test_state();
        let source = create(&state, "inherit-src-f").await;
        set_meta(&state, &source, serde_json::json!({ "window:theme": "nord" })).await;
        let stale = create(&state, "inherit-new-f").await;
        super::CREATED.lock().unwrap().insert(
            "inherit-new-f".to_string(),
            (stale.clone(), std::time::Instant::now() - super::JUST_CREATED),
        );

        inherit(&state, &source, "inherit-new-f").await;
        assert_eq!(meta(&state, &stale, "window:theme"), None);
        let new = create(&state, "inherit-new-f").await;
        assert_eq!(meta(&state, &new, "window:theme"), Some("nord".into()));
    }

    /// A source with no appearance of its own copies nothing.
    #[tokio::test]
    async fn a_default_source_copies_nothing() {
        let state = test_state();
        let source = create(&state, "inherit-src-e").await;
        inherit(&state, &source, "inherit-new-e").await;
        let new = create(&state, "inherit-new-e").await;
        assert_eq!(meta(&state, &new, "window:theme"), None);
        assert_eq!(meta(&state, &new, "window:opacity"), None);
    }

    #[tokio::test]
    async fn rejects_empty_args() {
        let state = test_state();
        let ret = handle_window_service(&state, &call("InheritWindowAppearance", vec!["".into(), "x".into()])).await;
        assert!(!ret.success);
    }
}
