// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `pins.list` / `pins.forget` / `pins.adopt`: the keys and installs this
//! machine has pinned to the names of agents that speak to it from elsewhere
//! (the LAN name pin, and the WAN install pin), and the way to move one.
//!
//! A pin decides who may speak as an agent, so changing one needs proof. The
//! operator's own window, proven by the host, may change any of them. An agent's
//! connection may list the pins (fingerprints only) and `adopt` a refused LAN key,
//! but only when srv itself finds the key vouched for by the install that hosts the
//! agent (`lan_continuity`, SPEC_LAN_KEY_CONTINUITY_2026_10_29.md): the caller's
//! word counts for nothing. `forget` stays with the window: forgetting a pin makes
//! the next key shown for the name first-seen, which is exactly how an impostor
//! would get in, and it has no proof to ask for. Every change is logged to
//! `logs/pin-changes.log`, with who made it.
//! `forget` drops a pin so the next key or install shown for the name is
//! pinned; `adopt` pins the key or install that was last refused for it.

use serde_json::json;

use super::*;
use crate::backend::rpc_types::{
    CommandPinsChangeData, CommandPinsListData, LanPinInfo, PinnedSendersResult, RefusedInstanceInfo, RefusedKeyInfo, WanPinInfo,
    COMMAND_PINS_ADOPT, COMMAND_PINS_FORGET, COMMAND_PINS_LIST,
};
use crate::server::reactive::lan_continuity::{self, fingerprint};
use crate::server::reactive::lan_refusals;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let st = state.clone();
    engine.register_typed(COMMAND_PINS_LIST, move |_req: CommandPinsListData, ctx| {
        let state = st.clone();
        async move {
            // Fingerprints only, never a key: an agent may read them.
            if ctx.agent_id.is_empty() {
                operator(&ctx)?;
            }
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
            if ctx.agent_id.is_empty() {
                operator(&ctx)?;
                change(&state, &req, Change::Adopt)?;
            } else {
                agent_adopt(&state, &ctx, &req).await?;
            }
            list(&state)
        }
    });
}

/// An agent's `adopt` of a refused LAN key: done only if the install that hosts the
/// agent vouches for it and this machine trusts that install, found by srv now.
async fn agent_adopt(state: &AppState, ctx: &RpcContext, req: &CommandPinsChangeData) -> Result<(), String> {
    if req.kind != "lan" {
        return Err("pins: an agent adopts a LAN key only; an install pin needs the user's window".to_string());
    }
    let agent = req.agent_id.trim().to_lowercase();
    if agent.is_empty() {
        return Err("pins: no agent named".to_string());
    }
    let refused = lan_refusals::get(&agent).ok_or_else(|| format!("pins: no other key has been shown for {agent} since AgentMux started"))?;
    confirm_shown(req.expect.as_deref(), &fingerprint(&refused.key_b64), &agent)?;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let key = BASE64.decode(&refused.key_b64).map_err(|_| "pins: the refused key is not a key".to_string())?;
    lan_continuity::move_if_vouched(state, &agent, &key, refused.cert.as_ref(), &ctx.agent_id, true)
        .await
        .map(|_| ())
        .map_err(|why| format!("pins: not vouched for, so not accepted: {why}"))
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
        "lan" => change_lan(state, &agent, how, req.expect.as_deref())?,
        "wan" => change_wan(&agent, req.channel.as_deref().ok_or("pins: a WAN pin needs its channel")?, how, req.expect.as_deref())?,
        other => return Err(format!("pins: unknown kind {other:?} (lan or wan)")),
    };
    lan_continuity::audit(&json!({ "action": how.as_str(), "kind": req.kind, "agent": agent, "channel": req.channel, "from": done.0, "to": done.1, "by": "operator" }));
    Ok(())
}

/// What a change moved: the pin before and after, as screen labels.
type Moved = (Option<String>, Option<String>);

/// An adoption is of what the screen showed: `shown` must be the refused key or
/// install srv holds now, or something else was refused in between.
fn confirm_shown(expect: Option<&str>, held: &str, agent: &str) -> Result<(), String> {
    match expect {
        Some(shown) if shown == held => Ok(()),
        Some(_) => Err(format!("pins: what was refused for {agent} has changed since the list; look at it again")),
        None => Err("pins: say which key or install is being accepted".to_string()),
    }
}

fn change_lan(state: &AppState, agent: &str, how: Change, expect: Option<&str>) -> Result<Moved, String> {
    let store_err = |e: crate::backend::storage::error::StoreError| format!("pins: {e}");
    match how {
        Change::Forget => {
            let old = state
                .mstore
                .lan_peer_pin_forget(agent)
                .map_err(store_err)?
                .ok_or_else(|| format!("pins: no LAN pin for {agent}"))?;
            lan_refusals::clear(agent);
            Ok((Some(fingerprint(&old)), None))
        }
        Change::Adopt => {
            let refused = lan_refusals::get(agent).ok_or_else(|| format!("pins: no other key has been shown for {agent} since AgentMux started"))?;
            confirm_shown(expect, &fingerprint(&refused.key_b64), agent)?;
            let old = state.mstore.lan_peer_pubkey_pin_replace(agent, &refused.key_b64).map_err(store_err)?;
            lan_refusals::clear(agent);
            Ok((old.map(|k| fingerprint(&k)), Some(fingerprint(&refused.key_b64))))
        }
    }
}

fn change_wan(agent: &str, channel: &str, how: Change, expect: Option<&str>) -> Result<Moved, String> {
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
            confirm_shown(expect, &label(&c.host_hint, &c.instance_id), agent)?;
            wan.agent_instance_pin_replace(agent, channel, &c.instance_id, &c.host_hint, agentmux_common::time::now_secs())
                .map_err(store_err)?;
            Ok((from, Some(label(&c.host_hint, &c.instance_id))))
        }
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
    async fn only_the_proven_operator_window_changes_pins_and_an_agent_only_adopts_with_proof() {
        let state = crate::server::tests::test_state();
        let forget = json!({ "kind": "lan", "agent_id": "maop" });
        for (command, data) in [(COMMAND_PINS_LIST, json!({})), (COMMAND_PINS_FORGET, forget.clone()), (COMMAND_PINS_ADOPT, forget.clone())] {
            let (unproven, _) = call(&state, None, command, data.clone()).await;
            assert!(unproven.starts_with("FORBIDDEN") && unproven.contains("isn't the AgentMux window"), "{command}: {unproven}");
        }
        let agent = || Some(RpcContext { agent_id: "AgentX".into(), ..Default::default() });
        // Forgetting a pin has no proof to ask for, so it stays with the window.
        let (err, _) = call(&state, agent(), COMMAND_PINS_FORGET, forget.clone()).await;
        assert!(err.starts_with("FORBIDDEN"), "{err}");
        // Listing is fingerprints only; adopting needs the vouch, and says what was missing.
        let (err, _) = call(&state, agent(), COMMAND_PINS_LIST, json!({})).await;
        assert_eq!(err, "");
        let (err, _) = call(&state, agent(), COMMAND_PINS_ADOPT, forget.clone()).await;
        assert!(!err.starts_with("FORBIDDEN") && err.contains("no other key has been shown"), "{err}");
        let (err, _) = call(&state, agent(), COMMAND_PINS_ADOPT, json!({ "kind": "wan", "agent_id": "maop", "channel": "stable" })).await;
        assert!(err.contains("LAN key only"), "{err}");
    }

    #[tokio::test]
    async fn an_agent_adopts_a_refused_lan_key_only_when_the_install_that_hosts_it_vouches() {
        use agentmux_common::jekt_sign::{generate_lan_keypair, WanKeyRecord};
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        let state = crate::server::tests::test_state();
        let tmp = |name: &str| {
            let dir = tempfile::tempdir().unwrap();
            let wan = std::sync::Arc::new(crate::backend::storage::wan_identity::WanIdentityStore::open(&dir.path().join(name)).unwrap());
            (dir, wan)
        };
        let (_sdir, sender) = tmp("sender.db");
        let (_rdir, receiver) = tmp("receiver.db");
        let host = sender.instance_ensure("starpower").unwrap();
        receiver.instance_ensure("narko").unwrap();
        receiver.agent_instance_pin_get_or_set("clare", "stable", &host.instance_id, &host.host_hint, agentmux_common::time::now_secs()).unwrap();
        state.mstore.set_wan_identity(receiver);

        let (old, _) = generate_lan_keypair([1u8; 32]);
        let (new, _) = generate_lan_keypair([2u8; 32]);
        let (new_b64, old_b64) = (BASE64.encode(new), BASE64.encode(old));
        state.mstore.lan_peer_pubkey_pin_get_or_set("pinagent", &old_b64).unwrap();
        let agent = || Some(RpcContext { agent_id: "AgentX".into(), ..Default::default() });
        let adopt = json!({ "kind": "lan", "agent_id": "pinagent", "expect": fingerprint(&new_b64) });

        // Refused with no certificate: not accepted, and the pin stays.
        lan_refusals::pin_mismatch_with("pinagent", &new_b64, None, Some("no certificate".into()));
        let (err, _) = call(&state, agent(), COMMAND_PINS_ADOPT, adopt.clone()).await;
        assert!(err.contains("not vouched for") && err.contains("no certificate"), "{err}");
        assert_eq!(state.mstore.lan_peer_pubkey_pin_load("pinagent").unwrap().as_deref(), Some(old_b64.as_str()));

        // Refused with the hosting install's certificate: accepted, and the audit line names the agent that asked.
        let vouch = WanKeyRecord::certify(&host.private_key, "pinagent", "lan", &new, &host.host_hint, agentmux_common::time::now_secs() + 5).unwrap();
        lan_refusals::pin_mismatch_with("pinagent", &new_b64, Some(vouch), None);
        let (err, _) = call(&state, agent(), COMMAND_PINS_ADOPT, adopt).await;
        assert_eq!(err, "");
        assert_eq!(state.mstore.lan_peer_pubkey_pin_load("pinagent").unwrap().as_deref(), Some(new_b64.as_str()));
        let audited = lan_continuity::AUDITED.lock().unwrap().iter().any(|e| e["agent"] == "pinagent" && e["by"] == "AgentX" && e["vouched_by"]["path"] == "pinned-instance");
        assert!(audited, "who accepted what, and why");
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

        // Adopt what the screen showed: nothing, or another key, is refused.
        let (err, _) = call(&state, op(), COMMAND_PINS_ADOPT, json!({ "kind": "lan", "agent_id": "pinops" })).await;
        assert!(err.contains("say which key"), "{err}");
        lan_refusals::pin_mismatch("pinops", "key-c"); // something else is refused after the list was drawn
        let (err, _) = call(&state, op(), COMMAND_PINS_ADOPT, json!({ "kind": "lan", "agent_id": "pinops", "expect": fingerprint("key-b") })).await;
        assert!(err.contains("has changed since the list"), "{err}");
        assert_eq!(state.mstore.lan_peer_pubkey_pin_load("pinops").unwrap().as_deref(), Some("key-a"), "nothing moved");
        lan_refusals::pin_mismatch("pinops", "key-b");
        let (err, _) = call(&state, op(), COMMAND_PINS_ADOPT, json!({ "kind": "lan", "agent_id": "PinOps", "expect": fingerprint("key-b") })).await;
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
