// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `pins.list` / `pins.forget` / `pins.adopt`: the keys and installs this
//! machine has pinned to the names of agents that speak to it from elsewhere
//! (the LAN name pin, and the WAN install pin), and the way to move one.
//!
//! A pin decides who may speak as an agent, so only the operator's own window,
//! proven by the host, may list or change them: a connection registered as an
//! agent may do neither. Every change is logged to `logs/pin-changes.log`.
//! `forget` drops a pin so the next key or install shown for the name is
//! pinned; `adopt` pins the key or install that was last refused for it.

use serde_json::json;
use sha2::{Digest, Sha256};

use super::*;
use crate::backend::rpc_types::{
    CommandPinsChangeData, CommandPinsListData, LanPinInfo, PinnedSendersResult, RefusedInstanceInfo, RefusedKeyInfo, WanPinInfo,
    COMMAND_PINS_ADOPT, COMMAND_PINS_FORGET, COMMAND_PINS_LIST,
};
use crate::server::reactive::lan_refusals;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let st = state.clone();
    engine.register_typed(COMMAND_PINS_LIST, move |_req: CommandPinsListData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            list(&state)
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_PINS_FORGET, move |req: CommandPinsChangeData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            change(&state, &req, Change::Forget)?;
            list(&state)
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_PINS_ADOPT, move |req: CommandPinsChangeData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            change(&state, &req, Change::Adopt)?;
            list(&state)
        }
    });
}

fn operator(ctx: &RpcContext) -> Result<(), String> {
    ctx.operator_only("only the user lists or changes the keys and installs pinned to agent names")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Forget,
    Adopt,
}

impl Change {
    fn as_str(self) -> &'static str {
        match self {
            Change::Forget => "forget",
            Change::Adopt => "adopt",
        }
    }
}

/// A short fingerprint of a base64 key: enough to tell two keys apart on a
/// screen, never the key.
fn fingerprint(key_b64: &str) -> String {
    hex::encode(&Sha256::digest(key_b64.as_bytes())[..5])
}

fn list(state: &AppState) -> Result<PinnedSendersResult, String> {
    let lan = state
        .mstore
        .lan_peer_pins_list()
        .map_err(|e| format!("pins: could not read the LAN pins: {e}"))?
        .into_iter()
        .map(|(agent_id, key, pinned_at, uid_pins)| {
            let refused = lan_refusals::get(&agent_id)
                .filter(|r| r.key_b64 != key)
                .map(|r| RefusedKeyInfo { key_fp: fingerprint(&r.key_b64), last_seen_at: r.last_seen_at, count: r.count });
            LanPinInfo { key_fp: fingerprint(&key), agent_id, pinned_at, uid_pins, refused }
        })
        .collect();
    let wan = match crate::backend::storage::wan_identity::global() {
        Some(wan) => wan
            .agent_instance_pins_list()
            .map_err(|e| format!("pins: could not read the WAN pins: {e}"))?
            .into_iter()
            .map(|p| WanPinInfo {
                instance: label(&p.host_hint, &p.instance_id),
                refused: p.conflict.map(|c| RefusedInstanceInfo {
                    instance: label(&c.host_hint, &c.instance_id),
                    last_seen_at: c.last_seen_at,
                    count: c.count as u32,
                }),
                agent_id: p.agent_id,
                channel: p.channel,
                pinned_at: p.pinned_at,
            })
            .collect(),
        None => Vec::new(),
    };
    Ok(PinnedSendersResult { lan, wan })
}

fn label(host_hint: &str, instance_id: &str) -> String {
    agentmux_common::jekt_sign::wan_instance_display_label(host_hint, instance_id)
}

fn change(state: &AppState, req: &CommandPinsChangeData, how: Change) -> Result<(), String> {
    let agent = req.agent_id.trim().to_lowercase();
    if agent.is_empty() {
        return Err("pins: no agent named".to_string());
    }
    let done = match req.kind.as_str() {
        "lan" => change_lan(state, &agent, how)?,
        "wan" => change_wan(&agent, req.channel.as_deref().ok_or("pins: a WAN pin needs its channel")?, how)?,
        other => return Err(format!("pins: unknown kind {other:?} (lan or wan)")),
    };
    audit(&json!({ "action": how.as_str(), "kind": req.kind, "agent": agent, "channel": req.channel, "from": done.0, "to": done.1 }));
    Ok(())
}

/// What a change moved: the pin before and after, as screen labels.
type Moved = (Option<String>, Option<String>);

fn change_lan(state: &AppState, agent: &str, how: Change) -> Result<Moved, String> {
    let store_err = |e: crate::backend::storage::error::StoreError| format!("pins: {e}");
    match how {
        Change::Forget => {
            let existed = state.mstore.lan_peer_pin_forget(agent).map_err(store_err)?;
            if !existed {
                return Err(format!("pins: no LAN pin for {agent}"));
            }
            lan_refusals::clear(agent);
            Ok((None, None))
        }
        Change::Adopt => {
            let refused = lan_refusals::get(agent).ok_or_else(|| format!("pins: no other key has been shown for {agent} since AgentMux started"))?;
            let old = state.mstore.lan_peer_pubkey_pin_replace(agent, &refused.key_b64).map_err(store_err)?;
            lan_refusals::clear(agent);
            Ok((old.map(|k| fingerprint(&k)), Some(fingerprint(&refused.key_b64))))
        }
    }
}

fn change_wan(agent: &str, channel: &str, how: Change) -> Result<Moved, String> {
    let wan = crate::backend::storage::wan_identity::global().ok_or("pins: this install has no WAN identity")?;
    let store_err = |e: crate::backend::storage::error::StoreError| format!("pins: {e}");
    let pin = wan
        .agent_instance_pin_get(agent, channel)
        .map_err(store_err)?
        .ok_or_else(|| format!("pins: no WAN pin for {agent} on {channel}"))?;
    let from = Some(label(&pin.host_hint, &pin.instance_id));
    match how {
        Change::Forget => {
            wan.agent_instance_pin_forget(agent, channel).map_err(store_err)?;
            Ok((from, None))
        }
        Change::Adopt => {
            let c = pin.conflict.ok_or_else(|| format!("pins: no other install has spoken for {agent} on {channel}"))?;
            wan.agent_instance_pin_replace(agent, channel, &c.instance_id, &c.host_hint, agentmux_common::time::now_secs())
                .map_err(store_err)?;
            Ok((from, Some(label(&c.host_hint, &c.instance_id))))
        }
    }
}

/// One line to `logs/pin-changes.log`, best effort. Fingerprints and labels
/// only: never a key.
fn audit(entry: &serde_json::Value) {
    let line = json!({ "at": agentmux_common::time::now_secs(), "change": entry });
    tracing::warn!(change = %entry, "pin changed by the operator");
    let Ok(root) = agentmux_common::data_paths::agentmux_root() else { return };
    let dir = root.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pin-changes.log")) {
        use std::io::Write;
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    async fn call(state: &AppState, ctx: Option<RpcContext>, command: &'static str, data: Value) -> (String, Option<Value>) {
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, state);
        if let Some(ctx) = ctx {
            engine.set_rpc_context(ctx);
        }
        engine.handle_message(RpcMessage { command: command.into(), reqid: "r".into(), data: Some(data), ..Default::default() });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        (resp.error, resp.data)
    }

    #[tokio::test]
    async fn only_the_proven_operator_window_lists_or_changes_pins() {
        let state = crate::server::tests::test_state();
        let forget = json!({ "kind": "lan", "agent_id": "maop" });
        for (command, data) in [(COMMAND_PINS_LIST, json!({})), (COMMAND_PINS_FORGET, forget.clone()), (COMMAND_PINS_ADOPT, forget.clone())] {
            let (unproven, _) = call(&state, None, command, data.clone()).await;
            assert!(unproven.starts_with("FORBIDDEN") && unproven.contains("isn't the AgentMux window"), "{command}: {unproven}");
            let agent = RpcContext { agent_id: "AgentX".into(), ..Default::default() };
            let (as_agent, _) = call(&state, Some(agent), command, data).await;
            assert!(as_agent.starts_with("FORBIDDEN"), "{command}: {as_agent}");
        }
    }

    #[tokio::test]
    async fn the_operator_lists_forgets_and_adopts_a_lan_pin() {
        let state = crate::server::tests::test_state();
        state.mstore.lan_peer_pubkey_pin_get_or_set("pinops", "key-a").unwrap();
        lan_refusals::pin_mismatch("pinops", "key-b");
        let op = || Some(RpcContext::operator_for_tests());

        let (err, data) = call(&state, op(), COMMAND_PINS_LIST, json!({})).await;
        assert_eq!(err, "");
        let lan = &data.unwrap()["lan"];
        let row = lan.as_array().unwrap().iter().find(|r| r["agent_id"] == "pinops").unwrap().clone();
        assert_eq!(row["key_fp"], fingerprint("key-a"));
        assert_eq!(row["refused"]["key_fp"], fingerprint("key-b"), "{row}");
        assert!(!row.to_string().contains("key-a"), "never the key itself: {row}");

        let (err, _) = call(&state, op(), COMMAND_PINS_ADOPT, json!({ "kind": "lan", "agent_id": "PinOps" })).await;
        assert_eq!(err, "");
        assert_eq!(state.mstore.lan_peer_pubkey_pin_load("pinops").unwrap().as_deref(), Some("key-b"));
        let (err, _) = call(&state, op(), COMMAND_PINS_ADOPT, json!({ "kind": "lan", "agent_id": "pinops" })).await;
        assert!(err.contains("no other key has been shown"), "{err}");

        let (err, _) = call(&state, op(), COMMAND_PINS_FORGET, json!({ "kind": "lan", "agent_id": "pinops" })).await;
        assert_eq!(err, "");
        assert!(state.mstore.lan_peer_pubkey_pin_load("pinops").unwrap().is_none());
        let (err, _) = call(&state, op(), COMMAND_PINS_FORGET, json!({ "kind": "lan", "agent_id": "pinops" })).await;
        assert!(err.contains("no LAN pin"), "{err}");
    }

    #[tokio::test]
    async fn a_request_for_nothing_sensible_is_refused() {
        let state = crate::server::tests::test_state();
        let op = || Some(RpcContext::operator_for_tests());
        let (err, _) = call(&state, op(), COMMAND_PINS_FORGET, json!({ "kind": "usb", "agent_id": "x" })).await;
        assert!(err.contains("unknown kind"), "{err}");
        let (err, _) = call(&state, op(), COMMAND_PINS_FORGET, json!({ "kind": "lan", "agent_id": "  " })).await;
        assert!(err.contains("no agent named"), "{err}");
        let (err, _) = call(&state, op(), COMMAND_PINS_FORGET, json!({ "kind": "wan", "agent_id": "x" })).await;
        assert!(err.contains("needs its channel") || err.contains("no WAN identity"), "{err}");
    }
}
