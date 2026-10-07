// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The desktop's side of pairing a device with the viewer listener
//! (`backend::viewer`, agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.2): the QR, the
//! paired-devices list with Revoke, and an agent's "hide from paired
//! devices" flag. Reached over the full-key WebSocket only, like every RPC.

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_pair_start(engine, state);
    register_devices(engine, state);
    register_revoke(engine, state);
    register_agent_hidden(engine, state);
}

/// Why there is no QR to show: the viewer listener is not up.
pub(crate) const NOT_LISTENING: &str =
    "The viewer isn't listening on this network. Turn on LAN discovery to pair a device.";

/// The `agentmux://pair` URL for a fresh code (cancelling the last one).
pub(crate) fn pair_start_impl(state: &AppState) -> Result<ViewerPairStartResult, String> {
    let (Some(port), Some(host)) = (state.viewer.advertised_port(), state.viewer.pair_host()) else {
        return Err(NOT_LISTENING.to_string());
    };
    let fingerprint = state.viewer.tls()?.fingerprint.clone();
    let (code, ttl) = state.viewer.pairing.start();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("v", "1")
        .append_pair("host", &host.to_string())
        .append_pair("port", &port.to_string())
        .append_pair("fp", &fingerprint)
        .append_pair("code", &code)
        .append_pair("hostname", &state.hostname)
        .append_pair("channel", &crate::backend::reactive::registry::local_channel_id())
        .finish();
    Ok(ViewerPairStartResult {
        url: format!("agentmux://pair?{query}"),
        expires_ms: agentmux_common::time::now_ms() + ttl.as_millis() as i64,
    })
}

fn register_pair_start(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_VIEWER_PAIR_START, move |_req: Option<NoArgsReq>, _ctx| {
        let state = state.clone();
        async move { pair_start_impl(&state) }
    });
}

fn register_devices(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_VIEWER_DEVICES, move |_req: Option<NoArgsReq>, _ctx| {
        let mstore = mstore.clone();
        async move {
            let devices = mstore
                .viewer_device_list()
                .map_err(|e| format!("viewer.devices: {e}"))?
                .into_iter()
                .map(|d| ViewerDeviceInfo {
                    device_id: d.device_id,
                    device_name: d.device_name,
                    created_ms: d.created_ms,
                    last_seen_ms: d.last_seen_ms,
                })
                .collect();
            Ok(ViewerDevicesResult { devices })
        }
    });
}

/// Revoke a device: its token stops working on the next request, and every
/// feed it has open is closed now.
pub(crate) fn revoke_impl(state: &AppState, device_id: &str) -> Result<ViewerRevokeResult, String> {
    let revoked = state.mstore.viewer_device_delete(device_id).map_err(|e| format!("viewer.revoke: {e}"))?;
    let feeds_closed = state.viewer.close_device_feeds(device_id);
    state.viewer.forget_device(device_id);
    if revoked {
        tracing::info!(%device_id, feeds_closed, "viewer: device revoked");
    }
    Ok(ViewerRevokeResult { revoked, feeds_closed })
}

fn register_revoke(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_VIEWER_REVOKE, move |cmd: CommandViewerRevokeData, _ctx| {
        let state = state.clone();
        async move { revoke_impl(&state, &cmd.device_id) }
    });
}

fn register_agent_hidden(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_VIEWER_AGENT_HIDDEN, move |cmd: CommandViewerAgentHiddenData, _ctx| {
        let mstore = mstore.clone();
        async move {
            if let Some(hidden) = cmd.hidden {
                let found = mstore
                    .agent_hide_from_devices_set(&cmd.agent_id, hidden)
                    .map_err(|e| format!("viewer.agent-hidden: {e}"))?;
                if !found {
                    return Err(format!("viewer.agent-hidden: no agent {}", cmd.agent_id));
                }
            }
            let hidden = mstore
                .agent_hide_from_devices_get(&cmd.agent_id)
                .map_err(|e| format!("viewer.agent-hidden: {e}"))?;
            Ok(ViewerAgentHiddenResult { hidden })
        }
    });
}
