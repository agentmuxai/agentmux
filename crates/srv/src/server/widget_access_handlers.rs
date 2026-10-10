// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The RPCs a sandboxed widget's pane host makes for it
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3, §11):
//! `widgets.session` when the pane opens, `widgets.endsession` when it
//! closes, and `widgets.call` for each bridge method srv answers (storage,
//! net, agents). Every call is checked here again against the session's
//! package and its grants (`backend/widget_access.rs`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::agent_state::{agent_status_of_block, AgentState};
use crate::backend::reactive::types::InjectionRequest;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::widget_access::{self, AccessError, STORAGE_MAX_BYTES, STORAGE_VALUE_MAX_BYTES, SEND_MAX_BYTES};
use crate::backend::widget_packages::{self, WidgetPackageInfo};
use crate::backend::widget_net;

use super::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetSessionReq {
    pub id: String,
    pub hash: String,
    pub blockid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetSessionResult {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetEndSessionReq {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetCallReq {
    pub token: String,
    pub method: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetCallResult {
    #[ts(type = "unknown")]
    pub result: Value,
}

fn packages() -> Vec<WidgetPackageInfo> {
    widget_packages::service().map(|s| s.list()).unwrap_or_default()
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed("widgets.session", |req: WidgetSessionReq, _ctx| async move {
        let token = widget_access::sessions().open(&packages(), &req.id, &req.hash, &req.blockid).map_err(|e| e.to_rpc())?;
        Ok(WidgetSessionResult { token })
    });

    engine.register_typed("widgets.endsession", |req: WidgetEndSessionReq, _ctx| async move {
        widget_access::sessions().close(&req.token);
        Ok(json!({}))
    });

    let st = state.clone();
    engine.register_typed("widgets.call", move |req: WidgetCallReq, _ctx| {
        let st = st.clone();
        async move { call(&st, &packages(), req).await.map(|result| WidgetCallResult { result }).map_err(|e| e.to_rpc()) }
    });
}

fn param<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str)
}

async fn call(st: &AppState, packages: &[WidgetPackageInfo], req: WidgetCallReq) -> Result<Value, AccessError> {
    let (_session, pkg) = widget_access::sessions().authorize(packages, &req.token)?;
    let p = &req.params;
    let store_err = |e: crate::backend::storage::error::StoreError| AccessError::internal(e.to_string());
    match req.method.as_str() {
        "storage.get" => {
            widget_access::require(&pkg, "storage")?;
            let key = param(p, "key").unwrap_or_default();
            widget_access::check_key(key)?;
            let value = st.mstore.widget_storage_get(&pkg.id, key).map_err(store_err)?;
            Ok(json!({ "value": value.and_then(|v| serde_json::from_str::<Value>(&v).ok()) }))
        }
        "storage.set" => {
            widget_access::require(&pkg, "storage")?;
            let key = param(p, "key").unwrap_or_default();
            widget_access::check_key(key)?;
            let value = p.get("value").ok_or_else(|| AccessError::invalid("storage.set needs { key, value }"))?;
            let text = value.to_string();
            if text.len() > STORAGE_VALUE_MAX_BYTES {
                return Err(AccessError::limit("storageValue", "a stored value is limited to 1 MB"));
            }
            if !st.mstore.widget_storage_set(&pkg.id, key, &text, STORAGE_MAX_BYTES).map_err(store_err)? {
                return Err(AccessError::limit("storage", "a widget's storage is limited to 5 MB"));
            }
            storage_changed(st, &pkg.id, key);
            Ok(json!({}))
        }
        "storage.delete" => {
            widget_access::require(&pkg, "storage")?;
            let key = param(p, "key").unwrap_or_default();
            widget_access::check_key(key)?;
            st.mstore.widget_storage_delete(&pkg.id, key).map_err(store_err)?;
            storage_changed(st, &pkg.id, key);
            Ok(json!({}))
        }
        "storage.list" => {
            widget_access::require(&pkg, "storage")?;
            let keys = st.mstore.widget_storage_keys(&pkg.id, param(p, "prefix").unwrap_or_default()).map_err(store_err)?;
            Ok(json!({ "keys": keys }))
        }
        "net.fetch" => {
            let fetch: widget_net::FetchRequest =
                serde_json::from_value(p.clone()).map_err(|e| AccessError::invalid(format!("net.fetch: {e}")))?;
            let resp = widget_net::fetch(&pkg.name, &pkg.granted, fetch).await?;
            serde_json::to_value(resp).map_err(|e| AccessError::internal(e.to_string()))
        }
        "agents.list" => {
            widget_access::require(&pkg, "agents:read")?;
            Ok(json!({ "agents": agent_rows(st) }))
        }
        "agents.send" => {
            widget_access::require(&pkg, "agents:send")?;
            let agent = param(p, "agent").unwrap_or_default();
            let text = param(p, "text").unwrap_or_default();
            if agent.is_empty() || text.is_empty() {
                return Err(AccessError::invalid("agents.send needs { agent, text }"));
            }
            if text.len() > SEND_MAX_BYTES {
                return Err(AccessError::limit("message", "a message from a widget is limited to 8 KB"));
            }
            if !st.reactive_handler.list_agents().iter().any(|a| a.agent_id.eq_ignore_ascii_case(agent)) {
                return Err(AccessError::not_found(format!("no agent named {agent} is running on this machine")));
            }
            widget_access::sessions().take_send(&pkg.id)?;
            let id = uuid::Uuid::new_v4().to_string();
            // As a message from the widget, with no signature: the agent
            // reads TRUST=self-declared and treats it like any unverified
            // message.
            let resp = st.reactive_handler.inject_message(InjectionRequest {
                target_agent: agent.to_string(),
                message: text.to_string(),
                source_agent: Some(format!("widget:{}", pkg.id)),
                request_id: Some(id.clone()),
                delivery_tier: Some("host".to_string()),
                ..Default::default()
            });
            if !resp.success {
                return Err(AccessError::internal(resp.error.unwrap_or_else(|| "the message wasn't delivered".to_string())));
            }
            tracing::info!(widget = %pkg.id, %agent, "widget sent a message to an agent");
            Ok(json!({ "id": id }))
        }
        other => Err(AccessError::new(-32601, format!("srv doesn't answer {other}"))),
    }
}

/// Tells the package's panes, in every window, which key changed (never its
/// value), so each can read it again.
fn storage_changed(st: &AppState, widget_id: &str, key: &str) {
    st.broker.publish(crate::backend::mps::MuxEvent {
        event: crate::backend::mps::EVENT_WIDGET_STORAGE.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: Some(json!({ "id": widget_id, "keys": [key] })),
    });
}

/// The user's running agents on this machine, with whether each is busy.
fn agent_rows(st: &AppState) -> Vec<Value> {
    let mut rows: Vec<Value> = st
        .reactive_handler
        .list_agents()
        .iter()
        .map(|a| {
            let state = match agent_status_of_block(&st.mstore, &a.block_id).map(|s| s.state) {
                Some(AgentState::Working) => "working",
                Some(AgentState::Stopped) | Some(AgentState::Error) => "stopped",
                _ => "idle",
            };
            json!({ "id": a.agent_id, "name": a.agent_id, "state": state })
        })
        .collect();
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::widget_packages::{WidgetKind, WidgetState};

    fn pkg(granted: &[&str]) -> WidgetPackageInfo {
        WidgetPackageInfo {
            id: "acme.calls".into(),
            name: "Calls".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            homepage: None,
            icon: "x".into(),
            default_hue: None,
            kind: WidgetKind::Sandboxed,
            permissions: granted.iter().map(|s| s.to_string()).collect(),
            granted: granted.iter().map(|s| s.to_string()).collect(),
            state: WidgetState::Approved,
            error: None,
            hash: "h1".into(),
            panes: vec![],
            commands: vec![],
            status_items: vec![],
            files_url: None,
            implied: false,
            folder: String::new(),
        }
    }

    async fn run(st: &AppState, packages: &[WidgetPackageInfo], token: &str, method: &str, params: Value) -> Result<Value, AccessError> {
        call(st, packages, WidgetCallReq { token: token.into(), method: method.into(), params }).await
    }

    // SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §11: srv refuses each
    // call whose permission wasn't granted, whatever the frontend checked.
    #[tokio::test]
    async fn srv_refuses_each_call_without_its_grant() {
        let st = crate::server::tests::test_state();
        let none = vec![pkg(&[])];
        let token = widget_access::sessions().open(&none, "acme.calls", "h1", "b1").unwrap();
        let cases = [
            ("storage.get", json!({ "key": "k" }), "storage"),
            ("storage.set", json!({ "key": "k", "value": 1 }), "storage"),
            ("storage.delete", json!({ "key": "k" }), "storage"),
            ("storage.list", json!({}), "storage"),
            ("agents.list", json!({}), "agents:read"),
            ("agents.send", json!({ "agent": "a", "text": "t" }), "agents:send"),
            ("net.fetch", json!({ "url": "https://api.github.com/user" }), "net:https://api.github.com"),
        ];
        for (method, params, permission) in cases {
            let e = run(&st, &none, &token, method, params).await.unwrap_err();
            assert_eq!(e.code, 1001, "{method}");
            assert_eq!(e.data, Some(json!({ "permission": permission })), "{method}");
        }

        // Granted: storage works, within the package, and its panes hear of it.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        st.broker.add_observer(Arc::new(move |ev| {
            if ev.event == crate::backend::mps::EVENT_WIDGET_STORAGE {
                log.lock().unwrap().push(ev.data.clone().unwrap_or_default());
            }
        }));
        let some = vec![pkg(&["storage", "agents:send"])];
        run(&st, &some, &token, "storage.set", json!({ "key": "note:1", "value": { "text": "hi" } })).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), [json!({ "id": "acme.calls", "keys": ["note:1"] })]);
        assert_eq!(run(&st, &some, &token, "storage.get", json!({ "key": "note:1" })).await.unwrap(), json!({ "value": { "text": "hi" } }));
        assert_eq!(run(&st, &some, &token, "storage.list", json!({ "prefix": "note:" })).await.unwrap(), json!({ "keys": ["note:1"] }));
        run(&st, &some, &token, "storage.delete", json!({ "key": "note:1" })).await.unwrap();
        assert_eq!(run(&st, &some, &token, "storage.get", json!({ "key": "note:1" })).await.unwrap(), json!({ "value": null }));
        // An agent that isn't running here.
        let e = run(&st, &some, &token, "agents.send", json!({ "agent": "nobody", "text": "hi" })).await.unwrap_err();
        assert_eq!(e.code, 1003);

        // The package changed: the session no longer answers.
        let mut changed = some.clone();
        changed[0].hash = "h2".into();
        assert_eq!(run(&st, &changed, &token, "storage.list", json!({})).await.unwrap_err().code, 1005);
        widget_access::sessions().close(&token);
    }
}
