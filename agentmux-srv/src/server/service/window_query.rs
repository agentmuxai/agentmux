// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handlers — read-only lookups (`GetWindow`,
//! `FindWindowByLabel`). Split out of `window.rs`; see that file's
//! dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;

pub(crate) async fn handle_get_window(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    match store.must_get::<Window>(&window_id) {
        Ok(win) => WebReturnType::success(serde_json::to_value(&win).unwrap_or_default()),
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

// SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11 Residual 2 —
// resolve Window rows by the `host:label` meta crumb written at
// CreateWindow time. Returns ALL matching window ids (labels can
// recur across host restarts — the crumb is a hint, not an
// identity); the caller decides what a multi-match means for its
// use case. An empty array is a normal answer (row predates the
// crumb, or the label never created a row).
pub(crate) async fn handle_find_window_by_label(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let label: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if label.is_empty() {
        return WebReturnType::error("FindWindowByLabel: empty label".to_string());
    }
    match store.get_all::<Window>() {
        Ok(wins) => {
            let ids: Vec<String> = wins
                .into_iter()
                .filter(|w| {
                    w.meta.get("host:label").and_then(|v| v.as_str())
                        == Some(label.as_str())
                })
                .map(|w| w.oid)
                .collect();
            WebReturnType::success(serde_json::json!(ids))
        }
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

#[cfg(test)]
mod window_label_crumb_tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn create_call(workspace_id: &str, label: Option<&str>) -> WebCallType {
        let mut args = vec![
            serde_json::Value::Null,
            serde_json::Value::String(workspace_id.to_string()),
        ];
        if let Some(l) = label {
            args.push(serde_json::Value::String(l.to_string()));
        }
        WebCallType {
            service: "window".to_string(),
            method: "CreateWindow".to_string(),
            uicontext: None,
            args,
        }
    }

    fn find_call(label: &str) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "FindWindowByLabel".to_string(),
            uicontext: None,
            args: vec![serde_json::Value::String(label.to_string())],
        }
    }

    /// The crumb round-trip this exists for: CreateWindow with a label arg
    /// persists `host:label` on the row, and FindWindowByLabel resolves it
    /// back to exactly that window id — no host-side registration involved.
    #[tokio::test]
    async fn crumb_written_at_create_and_resolvable_by_label() {
        let state = test_state();
        let ret = handle_window_service(
            &state,
            &create_call("", Some("window-pool-crumbtest1")),
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        let created_id = ret
            .data
            .as_ref()
            .and_then(|d| d.get("oid"))
            .and_then(|v| v.as_str())
            .expect("CreateWindow returns the Window with oid")
            .to_string();

        // The crumb is on the persisted row itself.
        let row = state
            .wstore
            .must_get::<Window>(&created_id)
            .expect("created window row exists");
        assert_eq!(
            row.meta.get("host:label").and_then(|v| v.as_str()),
            Some("window-pool-crumbtest1"),
            "host:label crumb missing from the Window row meta"
        );

        // And resolvable through the read RPC.
        let found = handle_window_service(&state, &find_call("window-pool-crumbtest1")).await;
        assert!(found.success, "FindWindowByLabel failed: {:?}", found.error);
        let ids: Vec<String> =
            serde_json::from_value(found.data.expect("find returns ids")).unwrap();
        assert_eq!(ids, vec![created_id]);
    }

    /// Old two-arg callers keep working and write no crumb — the arg is
    /// genuinely optional, not silently defaulted to something matchable.
    #[tokio::test]
    async fn two_arg_create_window_writes_no_crumb() {
        let state = test_state();
        let ret = handle_window_service(&state, &create_call("", None)).await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        let created_id = ret
            .data
            .as_ref()
            .and_then(|d| d.get("oid"))
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let row = state.wstore.must_get::<Window>(&created_id).unwrap();
        assert!(
            !row.meta.contains_key("host:label"),
            "two-arg CreateWindow must not invent a crumb"
        );
    }

    /// A label nothing ever created is a normal empty answer, not an error
    /// (rows predating the crumb look exactly like this to consumers).
    #[tokio::test]
    async fn unknown_label_returns_empty_not_error() {
        let state = test_state();
        let found = handle_window_service(&state, &find_call("window-never-existed")).await;
        assert!(found.success);
        let ids: Vec<String> =
            serde_json::from_value(found.data.expect("find returns ids")).unwrap();
        assert!(ids.is_empty());
    }

    /// Empty label is a caller bug — reject loudly rather than matching
    /// every crumbless row's absent key semantics ambiguously.
    #[tokio::test]
    async fn empty_label_is_an_error() {
        let state = test_state();
        let found = handle_window_service(&state, &find_call("")).await;
        assert!(!found.success);
    }
}
