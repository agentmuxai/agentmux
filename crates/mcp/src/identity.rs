// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent identity at the MCP boundary, jekt ids and outgoing jekt / UI-automation signatures.
//! Split out of main.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2).

use super::*;

/// The calling agent's slug (its `AGENTMUX_AGENT_ID`), injected by AgentMux into
/// this MCP server's trusted environment. The App API identity/preset/memory
/// REST endpoints stamp their `agent_id` from this — the agent's own model
/// output cannot reach those endpoints (no auth key in the PTY) nor override
/// this value, so the slug cannot be forged. See
/// SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md §5.
pub(crate) fn agent_slug() -> Result<String> {
    let slug = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
    if slug.is_empty() {
        anyhow::bail!(
            "AGENTMUX_AGENT_ID is not set — cannot resolve this agent's identity. \
             Is this agent pane opened via AgentMux?"
        );
    }
    Ok(slug)
}

/// Identity M3 (spec §5): resolve a typed agent name at the boundary — the
/// only place a name is turned into a UID — before a tool stores it.
/// `Ok(Some(uid))` when exactly one identified agent matches; `Ok(None)` when
/// none does, the match has no UID, or the srv predates the endpoint (the
/// tool then sends the name and the server records the miss). `Err` in two
/// cases, both of which fail the tool call: the name is ambiguous — the
/// error lists the candidates so the model can retry by uid (§5.2) — or the
/// resolver could not answer (a store fault, any other non-2xx, an
/// unreadable or unexpected response), where sending the bare name would
/// skip the ambiguity check. See `interpret_resolve_response`.
pub(crate) async fn resolve_agent_name_at_boundary(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    name: &str,
) -> Result<Option<String>> {
    let url = format!("{}/agentmux/agents/resolve", local_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTH_KEY_HEADER, auth_key)
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("agent name resolve request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    interpret_resolve_response(status, &text, name)
}

/// The response half of `resolve_agent_name_at_boundary`, separated so the
/// status handling is testable without a server.
pub(crate) fn interpret_resolve_response(
    status: reqwest::StatusCode,
    text: &str,
    name: &str,
) -> Result<Option<String>> {
    // Only an srv that predates the endpoint (404/405) gets the
    // compatibility fallback of sending the bare name. Any other failure —
    // a store fault (503), a panicked handler (500) — fails the tool call:
    // proceeding by name would skip the ambiguity check and could hand the
    // item to an exact-same-name other (Codex P2 on #3563).
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return Ok(None);
    }
    if !status.is_success() {
        anyhow::bail!("could not resolve agent \"{name}\": HTTP {status} — {text}");
    }
    let v: Value = serde_json::from_str(text).map_err(|e| {
        anyhow::anyhow!("could not resolve agent \"{name}\": unreadable response ({e})")
    })?;
    match v.get("resolution").and_then(|r| r.as_str()).unwrap_or("") {
        "one" => match v
            .get("uid")
            .and_then(|u| u.as_str())
            .filter(|u| !u.is_empty())
        {
            Some(uid) => Ok(Some(uid.to_string())),
            None => anyhow::bail!(
                "could not resolve agent \"{name}\": a single match came back without a uid"
            ),
        },
        "ambiguous" => {
            let listed: Vec<String> = v
                .get("candidates")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|c| {
                            format!(
                                "  • {} (uid {}{}{})",
                                c.get("name").and_then(|x| x.as_str()).unwrap_or("?"),
                                c.get("uid").and_then(|x| x.as_str()).unwrap_or("none"),
                                c.get("block_id").and_then(|x| x.as_str()).map(|b| format!(", block {b}")).unwrap_or_default(),
                                if c.get("live").and_then(|x| x.as_bool()).unwrap_or(false) { ", live" } else { "" },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            anyhow::bail!(
                "Several agents match \"{name}\":\n{}\nAddress one directly by uid, or rename one.",
                listed.join("\n")
            )
        }
        "none" | "unidentified" => Ok(None),
        other => {
            anyhow::bail!("could not resolve agent \"{name}\": unexpected resolution {other:?}")
        }
    }
}

/// Process-wide counter for `generate_jekt_msgid` — millis-timestamp alone
/// isn't guaranteed unique if two jekts are sent in the same millisecond.
pub(crate) static JEKT_MSGID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique-enough message id for a signed jekt: no `uuid` dependency needed
/// (this crate doesn't otherwise pull one in) — timestamp + this process's
/// own pid + a monotonic counter is unique enough for its one purpose (a
/// value both this signer and the receiving srv agree to include in the
/// signed material, so a signature can't be replayed under a different id).
pub(crate) fn generate_jekt_msgid() -> String {
    let n = JEKT_MSGID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", agentmux_common::time::now_ms_u64(), std::process::id(), n)
}

/// `FleetBroadcast`'s block_id -> agent-name resolution
/// (REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md): a `/agentmux/discovery`
/// response's `host.addressable` AND `host.cross_channel` sections both
/// carry a `block_id` — `addressable` under `agent_id`/`block_id`,
/// `cross_channel` (a different channel on this same host) under
/// `name`/`block_id`. A block_id is unique across channels on one host, so
/// both are folded into one map, not kept separate. `lan`/`wan` entries
/// carry no `block_id` at all (they're not local blocks) — a target for
/// those tiers is never in this map, and the caller falls back to treating
/// it as a literal agent name instead (see the `FleetBroadcast` handler).
///
/// Pure (no I/O) — extracted so it's unit-testable without a live discovery
/// endpoint.
pub(crate) fn build_block_to_agent_map(discovery: &Value) -> std::collections::HashMap<String, String> {
    let mut map: std::collections::HashMap<String, String> = discovery
        .get("host")
        .and_then(|h| h.get("addressable"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let agent_id = e.get("agent_id")?.as_str()?.to_string();
                    let block_id = e.get("block_id")?.as_str()?.to_string();
                    Some((block_id, agent_id))
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(arr) = discovery.get("host").and_then(|h| h.get("cross_channel")).and_then(|v| v.as_array()) {
        for e in arr {
            if let (Some(name), Some(block_id)) = (
                e.get("name").and_then(|v| v.as_str()),
                e.get("block_id").and_then(|v| v.as_str()),
            ) {
                map.insert(block_id.to_string(), name.to_string());
            }
        }
    }
    map
}

/// Build the id/timestamp/host-signature/LAN-signature quadruple for an
/// outgoing jekt (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2,
/// SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.3). Reads this process's OWN
/// `AGENTMUX_JEKT_KEY`/`AGENTMUX_LAN_KEY` — injected at spawn alongside
/// `AGENTMUX_AGENT_ID`, into this agent's env only, never any other
/// agent's — and signs over the same (msgid, source_agent, target_agent,
/// ts_secs, message) material both schemes share.
///
/// Both signatures are computed unconditionally, regardless of which
/// delivery tier the message actually ends up taking — this process has no
/// reliable way to know that in advance (routing/forwarding is a
/// server-side decision, see
/// `docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §3). Sending a
/// `lan_sig` alongside a message that's actually delivered host or WAN
/// costs nothing — srv only ever consults `lan_sig` when it has
/// independently determined `delivery_tier == "lan"` (never trusting the
/// message body's own claim), so an irrelevant signature is simply ignored.
///
/// Returns `(request_id, ts_secs, jekt_sig, lan_sig)`. Either signature is
/// `None` — not an error — when its key is unavailable (an agent whose
/// `.mcp.json` predates this feature, or `source_agent` itself unresolved):
/// srv treats an absent signature as "unverified," never as a reason to
/// fail delivery, so a missing key here must never block
/// `SendMessage`/`Loop` from sending.
///
/// The cross-channel signature (`channel_sig`,
/// `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md` §D5) rides along on the
/// same terms as `lan_sig`: same `AGENTMUX_LAN_KEY`, but over a
/// domain-separated payload that also binds this process's channel
/// (`AGENTMUX_CHANNEL`, injected into this env at spawn by
/// `inject_jekt_signing_keys_into_mcp_json`; defaults to `stable` exactly as
/// srv's own registry writer does). srv only consults it once it has itself
/// labelled the delivery `channel` — a same-machine forward to a different
/// instance — so on every other tier it's simply ignored.
///
/// Returned as a named struct rather than a tuple: the three signatures are
/// consecutive `Option<String>`s and a transposition at any of the call
/// sites would compile clean and silently mislabel trust on the receiver.
/// The WAN signature (`wan_sig`) rides along on exactly the
/// same terms as `lan_sig`, for the same reason: this process cannot know
/// which tier a send will take — it posts every send to the local reactive
/// endpoint and srv decides among local, cross-channel, LAN and cloud
/// afterwards. Computing it unconditionally is therefore the only
/// implementable shape, and an irrelevant signature costs nothing because srv
/// only consults `wan_sig` once it has itself determined the delivery is WAN.
/// It signs a domain-separated payload under this agent's own
/// `AGENTMUX_WAN_KEY`, which is a different key from `AGENTMUX_LAN_KEY`.
pub(crate) struct OutgoingJektSignatures {
    request_id: String,
    ts_secs: i64,
    jekt_sig: Option<String>,
    lan_sig: Option<String>,
    source_channel: Option<String>,
    channel_sig: Option<String>,
    wan_sig: Option<String>,
    wan_source_host: Option<String>,
}

impl OutgoingJektSignatures {
    /// The wire request these signatures were minted for — every send path
    /// builds it through here so none can forget a field.
    pub(crate) fn into_request(self, target_agent: String, message: String, source_agent: Option<String>) -> InjectRequest {
        InjectRequest {
            target_agent,
            message,
            source_agent,
            request_id: Some(self.request_id),
            ts_secs: Some(self.ts_secs),
            jekt_sig: self.jekt_sig,
            lan_sig: self.lan_sig,
            source_channel: self.source_channel,
            channel_sig: self.channel_sig,
            wan_sig: self.wan_sig,
            wan_source_host: self.wan_source_host,
        }
    }
}

pub(crate) fn sign_outgoing_jekt(
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
) -> OutgoingJektSignatures {
    let msgid = generate_jekt_msgid();
    let ts_secs = agentmux_common::time::now_secs();
    // The keys: fetched from srv when they're this agent's, else the env's
    // (identity M4d-3, self_keys.rs).
    let jekt_sig = (|| {
        let key = crate::self_keys::jekt_key()?;
        let src = source_agent?;
        Some(agentmux_common::jekt_sign::sign_jekt(&key, &msgid, src, target_agent, ts_secs, message))
    })();
    let lan_key = crate::self_keys::lan_key();
    let lan_sig = (|| {
        let key = lan_key.as_deref()?;
        let src = source_agent?;
        agentmux_common::jekt_sign::sign_lan_jekt(key, &msgid, src, target_agent, ts_secs, message)
    })();
    let source_channel = std::env::var("AGENTMUX_CHANNEL").ok().filter(|s| !s.is_empty());
    let channel_sig = (|| {
        let key = lan_key.as_deref()?;
        let src = source_agent?;
        // Same default srv's registry writer uses (`local_channel_id`): an
        // unset var means the stable channel, not "unknown".
        let channel = source_channel.as_deref().unwrap_or("stable");
        agentmux_common::jekt_sign::sign_channel_jekt(key, &msgid, src, channel, target_agent, ts_secs, message)
    })();
    // The WAN signature uses its own key, not
    // AGENTMUX_LAN_KEY. An agent whose `.mcp.json` predates this feature has
    // no AGENTMUX_WAN_KEY and simply sends unsigned, exactly as it does today
    // for every other tier.
    //
    // The WAN signature binds this agent's INSTANCE — host and channel — not
    // just its name (§2.1.2): one account can run the same agent name on
    // several machines, and on several build channels of one machine, and
    // each instance mints its own keypair because each has its own database.
    // The instance is what selects which published key a receiver checks
    // against, so it has to be inside the signature.
    let source_host = std::env::var("AGENTMUX_HOST_LABEL").ok().filter(|s| !s.is_empty());
    let wan_sig = (|| {
        let key = crate::self_keys::wan_key()?;
        let src = source_agent?;
        // Same defaulting discipline as `channel` above: an unset var must
        // resolve to the identical string srv publishes under, never to a
        // second spelling of "unknown".
        let host = source_host.as_deref().unwrap_or("unknown");
        let channel = source_channel.as_deref().unwrap_or("stable");
        agentmux_common::jekt_sign::sign_wan_jekt(
            &key, &msgid, src, host, channel, target_agent, ts_secs, message,
        )
    })();
    // Only declare an instance component when some signature is bound to it —
    // a bare label with nothing to verify is noise on the wire. `wan_sig` now
    // binds the channel too, so the channel ships when EITHER signature
    // exists, not only the cross-channel one.
    let wan_source_host = wan_sig
        .as_ref()
        .map(|_| source_host.unwrap_or_else(|| "unknown".to_string()));
    let source_channel = (channel_sig.is_some() || wan_sig.is_some())
        .then(|| source_channel.unwrap_or_else(|| "stable".to_string()));
    OutgoingJektSignatures {
        request_id: msgid,
        ts_secs,
        jekt_sig,
        lan_sig,
        source_channel,
        channel_sig,
        wan_sig,
        wan_source_host,
    }
}

/// Build the identity proof every `/api/v1/ui/*` request carries — an
/// HMAC-SHA256 signature over this agent's own agent_id, using this
/// agent's own `AGENTMUX_JEKT_KEY` (the same per-agent key `sign_outgoing_jekt`
/// above uses for jekt messages; reused rather than inventing a parallel
/// credential system). Unlike jekt signing, a missing key here is a hard
/// error, not a silent "send unsigned" — srv has no unverified fallback
/// path for UI automation (see `crates/srv/src/server/ui_handlers.rs`'s
/// module doc comment), so a tool call with no key would just 401 anyway;
/// failing fast with a clear "respawn to get a key" message is more useful
/// than a confusing round trip.
pub(crate) fn sign_ui_automation_auth() -> Result<agentmux_common::api_types::UiAutomationAuth> {
    let agent_id = agent_slug()?;
    // The jekt key it signs v1 with (identity M4d-3): fetched when ours.
    let key = crate::self_keys::jekt_key().ok_or_else(|| {
        anyhow::anyhow!(
            "AGENTMUX_JEKT_KEY is not set (or not valid base64) and srv served no key — this agent \
             needs to be respawned to get a signing key before it can use UI automation \
             (UIScreenshot/UIClick/UIQuery)"
        )
    })?;
    let ts_secs = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(
        &key,
        "ui-automation-identity",
        &agent_id,
        "__srv__",
        ts_secs,
        "",
    );
    Ok(agentmux_common::api_types::UiAutomationAuth { agent_id, ts_secs, sig })
}
