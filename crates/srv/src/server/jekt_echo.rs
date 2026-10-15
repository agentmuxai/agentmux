// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sender's own copy of a jekt it sent, written into its pane
//! (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2). Moved out of `reactive.rs`.

use super::AppState;

/// Echo a successfully-sent jekt into the SENDER's own pane
/// (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
///
/// Appends a `{"type":"user",...}` NDJSON line carrying the same
/// `[JEKT:...]` marker block the receiver got (re-wrapped with identical
/// fields) to the sender's `output` blockfile — live MPS append (renders
/// immediately in an open agent view), persisted history
/// (`parseHistoryLines` rebuilds on reopen), and global transcript mirror.
/// The frontend's `tryParseJekt` sees FROM == this pane's agent and renders
/// it as an *outgoing* JektBubble (stream-parser.ts direction detection —
/// this is the producer that comment says doesn't exist yet).
///
/// No-op when the sender isn't a registered agent on this instance (cron,
/// external callers) or is messaging itself (the incoming marker already
/// lands in the same pane).
///
/// `reagent_verified`/`lan_verified` should be the SAME values the original
/// delivery's `Handler::inject_message_inner` computed `effective_tier`/
/// `requires_stop` from (not re-derived) — see the call site's own doc
/// comment inline below for why passing anything else produces a
/// self-contradictory echoed marker (reagentx P1 on PR #2623).
/// The delivery-tier + verification signals an echoed marker must carry.
///
/// Grouped into a named struct rather than left as positionals because
/// `sig_verified`, `reagent_verified` and `lan_verified` are three consecutive
/// `Option<bool>`s: any two of them can be transposed at a call site and the
/// compiler cannot say a word. The failure mode is a silently mislabelled TRUST
/// field on a security marker, which is not the kind of bug to leave to
/// argument order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EchoTrust<'a> {
    pub delivery_tier: &'a str,
    pub sig_verified: Option<bool>,
    pub reagent_verified: Option<bool>,
    pub lan_verified: Option<bool>,
    pub channel_verified: Option<bool>,
}

/// The trust an echoed marker should carry after a forward, given the
/// peer's response body. The RECEIVING instance is the one that computes
/// `channel_verified` (the forwarder holds the sender's HMAC key, so §D2
/// step 2 of SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md makes it skip
/// cross-channel verification) and threads it back via
/// `InjectionResponse::channel_verified`, exactly as it already does for
/// `effective_tier`/`requires_stop`. Taking it from the body keeps the
/// sender's echoed `TRUST=` consistent with the `ESCALATE=` the same body
/// supplies — otherwise a sensitive cross-channel message could echo as
/// `TRUST=self-declared … ESCALATE=none`, which contradicts itself (codex
/// P2 on #3064). A body without the field (an older peer) leaves the
/// caller's value alone.
pub(super) fn echo_trust_from_peer_body<'a>(body: &serde_json::Value, base: EchoTrust<'a>) -> EchoTrust<'a> {
    EchoTrust {
        channel_verified: body
            .get("channel_verified")
            .and_then(|v| v.as_bool())
            .or(base.channel_verified),
        ..base
    }
}

pub(super) fn echo_jekt_to_sender(
    state: &AppState,
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
    msgid: &str,
    effective_tier: Option<&str>,
    requires_stop: Option<bool>,
    trust: EchoTrust<'_>,
    priority: &str,
) {
    echo_jekt_to_sender_with_files(state, source_agent, target_agent, message, msgid, effective_tier, requires_stop, trust, priority, None);
}

/// [`echo_jekt_to_sender`] for a jekt delivered with files: the sender's
/// echo lists them as the receiver's message did, so its pane shows the
/// same chips (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §6.2).
#[allow(clippy::too_many_arguments)]
pub(super) fn echo_jekt_to_sender_with_files(
    state: &AppState,
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
    msgid: &str,
    effective_tier: Option<&str>,
    requires_stop: Option<bool>,
    trust: EchoTrust<'_>,
    priority: &str,
    files: Option<&crate::backend::reactive::types::FilesDelivery>,
) {
    let EchoTrust {
        delivery_tier,
        sig_verified,
        reagent_verified,
        lan_verified,
        channel_verified,
    } = trust;
    let Some(src) = source_agent.filter(|s| !s.is_empty()) else {
        return;
    };
    if src.eq_ignore_ascii_case(target_agent) {
        return;
    }
    let Some(sender_reg) = state.reactive_handler.get_agent(src) else {
        return;
    };

    let sanitized = crate::backend::reactive::sanitize::sanitize_message(message);
    let wrapped = crate::backend::reactive::sanitize::wrap_jekt_message_with_files(
        &sanitized,
        Some(&sender_reg.agent_id),
        target_agent,
        effective_tier.unwrap_or("coord"),
        delivery_tier,
        sig_verified,
        // reagentx P1 on PR #2623: these used to be hardcoded `None, None`
        // ("sender-echo is inherently host-tier, so WAN/LAN verification is
        // meaningless here") — true for TRUST/SIG rendering alone, but as of
        // SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md
        // that assumption broke: `requires_stop` (just above) already
        // reflects the REAL delivery's verification, so hardcoding these to
        // `None` produced a self-contradictory echoed marker — `ESCALATE=none`
        // (implying a verified sender) next to `TRUST=network-claimed` with
        // no `SIG=` field (implying an unverified one). Passing the same
        // verification signals the original decision was made from keeps
        // the echoed marker internally consistent with its own `ESCALATE=`.
        reagent_verified,
        lan_verified,
        channel_verified,
        // The sender-side echo never has a WAN verdict: WAN verification
        // happens only at the receiving install (W3-S §2.3), after the echo.
        None,
        // Defaults to `true` (STOP) when the caller couldn't tell us —
        // matches `effective_tier` defaulting to the more-cautious "coord"
        // rather than assuming "info" above; never silently downgrades a
        // sensitive echo to tag-only just because this hop lost the signal.
        requires_stop.unwrap_or(true),
        msgid,
        priority,
        None,
        files,
    );
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": wrapped }
    });
    let data = format!("{line}\n");
    let global_zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(
        &Some(state.mstore.clone()),
        &sender_reg.block_id,
    );
    crate::backend::blockcontroller::shell::handle_append_block_file(
        &state.broker,
        &sender_reg.block_id,
        crate::backend::agent_session::OUTPUT_FILE,
        data.as_bytes(),
        Some(&state.filestore),
        global_zone.as_deref(),
    );
}
