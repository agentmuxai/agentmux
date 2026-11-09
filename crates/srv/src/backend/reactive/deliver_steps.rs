// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`Handler::deliver_audited`], one function each, in the order they ran: tidy the
//! request, resolve its target, confirm the recipient's identity, decide the jekt tier, then
//! deliver through the controller's structured channel or as keystrokes. Moved out of
//! `deliver_audited` without a change in behaviour.

use super::*;

/// What the tier rules decided for one message.
pub(super) struct TierDecision {
    pub effective_tier: &'static str,
    pub requires_stop: bool,
}

/// A refusal made before the tier is known: no tier fields on the response.
pub(super) fn early_refusal(request_id: String, block_id: Option<String>, error: String, now: u64) -> InjectionResponse {
    InjectionResponse {
        deferred: None,
        success: false,
        request_id,
        block_id,
        error: Some(error),
        timestamp: now,
        effective_tier: None,
        requires_stop: None,
        channel_verified: None,
    }
}

/// The facts of one delivery once its target and tier are settled.
pub(super) struct Delivery<'a> {
    pub req: &'a InjectionRequest,
    pub sanitized: &'a str,
    pub request_id: String,
    pub now: u64,
    pub block_id: String,
    pub effective_tier: &'static str,
    pub requires_stop: bool,
    pub delivery_tier: &'a str,
    pub outcome_on_success: Option<&'a str>,
    pub outcome_on_failure: Option<&'a str>,
}

impl Delivery<'_> {
    /// The response for this delivery: a success when `error` is `None`.
    fn response(&self, error: Option<String>, deferred: Option<bool>) -> InjectionResponse {
        InjectionResponse {
            deferred,
            success: error.is_none(),
            request_id: self.request_id.clone(),
            block_id: Some(self.block_id.clone()),
            error,
            timestamp: self.now,
            effective_tier: Some(self.effective_tier.to_string()),
            requires_stop: Some(self.requires_stop),
            channel_verified: self.req.channel_verified,
        }
    }
}

/// Only the trusted production key makes a verified reagent sender.
/// The verifiers already apply this; enforce it here too for any
/// caller that sets `reagent_verified` directly.
fn downgrade_untrusted_reagent(req: &mut InjectionRequest) {
    if req.reagent_verified == Some(true)
        && !req
            .reagent_key_id
            .as_deref()
            .is_some_and(agentmux_common::jekt_sign::is_reagent_trusted_signing_key)
    {
        req.reagent_verified = Some(false);
    }
}

// Determine effective jekt tier.
// Escalation rules (spec §5.2, extended by
// SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, by
// SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md §1 for the
// network-tier exception below, and NARROWED by
// SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md — repo-owner-
// confirmed directly in a live conversation, not a jekt/muxbus claim):
//   1. WAN delivery, a reagent signature was present but failed, or
//      was valid only under an untrusted key such as `reagent-v1-dev`
//      (SIG=invalid) → always SENSITIVE, regardless of declared tier
//      or keyword content. No reagent signature at all is NOT this
//      case; that is rule 5's default. Failed LAN and WAN-instance
//      checks are their own forcing rules below.
//   1b. WAN delivery, sender verified via reagent's pinned Ed25519
//      key (`reagent_verified == Some(true)`) → NOT forced to
//      SENSITIVE by delivery tier alone. As of the 2026-08-15
//      narrowing this is NOT a distinct check anymore — it's simply
//      rule 1 not matching (`Some(true)` isn't `Some(false)`).
//      `Some(true)` only ever means the trusted `reagent-v1` key:
//      the verifiers use `verify_trusted_reagent_jekt`, and
//      `deliver_audited` downgrades any other key to `Some(false)`.
//      Rules 3/4 below still apply on top: a verified reagent
//      message that declares SENSITIVE or matches the keyword scan
//      still escalates.
//   2. Host check failed, on any delivery tier (the claimed sender
//      has a host key on file and the signature is missing or wrong,
//      or it is a slug its agent was renamed from) → always
//      SENSITIVE (see `sig_verified` on `InjectionRequest` for
//      exactly when this applies vs. is skipped). Same "an active
//      verification FAILURE is the red flag, not mere absence of
//      one" logic as rule 1 — this rule was already scoped that way.
//   3. Declared SENSITIVE (any tier) → SENSITIVE.
//   4. Keyword match (any tier) → SENSITIVE.
//   5. Otherwise → use declared tier (default: coord). This is now
//      reachable by ordinary unverified LAN/WAN traffic with clean
//      content — `TRUST` in the marker is UNCHANGED by this
//      narrowing (still reads `network-claimed`, still exactly as
//      forgeable as ever); only whether that lack of proof alone
//      is sufficient grounds to interrupt the human has changed.
/// Decide the effective jekt tier and whether it stops the receiver.
pub(super) fn decide_tier(req: &InjectionRequest, sanitized: &str) -> TierDecision {
    let declared_tier = req.jekt_tier.as_ref();
    let delivery_tier = req.delivery_tier.as_deref().unwrap_or("host");
    let is_network_tier = delivery_tier == "wan" || delivery_tier == "lan";
    // A reagent_sig that was PRESENT but didn't verify — someone tried to
    // forge it. `reagent_verified` is WAN-only by construction
    // (`sync_agent_reactive`/`verify_reagent_signature` never compute it
    // off the WAN tier, so it's always `None` for LAN — reagent is a
    // WAN-only service sender, this never applied to LAN). A signature
    // that verified only under a key other than the trusted production
    // key IS this case (it arrives as `Some(false)`, see
    // `deliver_audited`); absence of a signature attempt (`None`) falls
    // through to rule 5 like any other self-declared sender.
    let is_network_tier_sig_invalid = is_network_tier && req.reagent_verified == Some(false);
    // A lan_sig that was PRESENT, whose claimed sender's public key WAS
    // found, but didn't cryptographically verify — a specific agent's
    // identity was actively forged, not merely unproven. Scoped to LAN
    // only, same reasoning as reagent's WAN scoping above — see
    // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.4/§2.5.
    let is_lan_sig_invalid = delivery_tier == "lan" && req.lan_verified == Some(false);
    let is_unverified_sender = req.sig_verified == Some(false);
    // W3-S (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.6): a same-account
    // agent's WAN signature that actively failed (envelope, certificate
    // chain, record match, signature or replay), or one from an instance
    // its owner has revoked — the key is known to be out of its owner's
    // control — is forced sensitive, like `SIG=invalid`.
    let wan_instance_status = req.wan_instance.as_ref().map(|i| i.status);
    let is_wan_sig_invalid = delivery_tier == "wan"
        && (req.wan_verified == Some(false)
            || (req.wan_verified == Some(true)
                && wan_instance_status == Some(crate::backend::reactive::types::WanInstanceStatus::Revoked)));
    // SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md rule 1: a
    // transcript_request (muxspect Phase B/C's LAN/WAN conversation-
    // visibility protocol) is forced sensitive unconditionally, on any
    // tier, regardless of trust — the existing credential/destructive-
    // keyword scan (is_sensitive_message) doesn't catch a content-
    // disclosure *request* at all. `req.is_transcript_request` is
    // server-computed in `handle_reactive_inject` by re-parsing
    // `message` itself (never attacker-settable — see its own doc
    // comment), so this can't be spoofed by a client claiming/denying it.
    let is_sensitive = is_network_tier_sig_invalid
        || is_lan_sig_invalid
        || is_wan_sig_invalid
        || is_unverified_sender
        || matches!(declared_tier, Some(crate::backend::reactive::types::JektTier::Sensitive))
        || is_sensitive_message(sanitized)
        // A file's name is scanned like its message (file transfer spec §6.4).
        || req.files.iter().flatten().any(|f| is_sensitive_message(&f.name))
        || req.is_transcript_request;
    let effective_tier = if is_sensitive { "sensitive" } else {
        declared_tier.map_or("coord", |t| match t {
            crate::backend::reactive::types::JektTier::Info => "info",
            crate::backend::reactive::types::JektTier::Coord => "coord",
            crate::backend::reactive::types::JektTier::Sensitive => "sensitive",
        })
    };
    let is_cryptographically_verified = is_cryptographically_verified(req, delivery_tier);
    // SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md rule 2: the
    // ONE named exception to the verified-sender relaxation above. A
    // transcript_request's ESCALATE=required is not relaxed by a
    // verified sender when the RESPONDING agent's own
    // conversation_visibility is `ask` (or `trusted_peers` with a
    // non-allow-listed requester) — `req.transcript_request_escalate_forced`,
    // resolved by the server (`resolve_transcript_request_tier_fields`)
    // against that agent's own setting before the marker is rendered,
    // and meaningful only with `is_transcript_request`. A valid signature
    // answers *who is asking*, not *whether to disclose this content* —
    // identity proof alone doesn't relax that question, unlike every
    // other TIER=sensitive case above.
    let requires_stop = is_sensitive
        && (!is_cryptographically_verified || req.transcript_request_escalate_forced);
    TierDecision { effective_tier, requires_stop }
}

/// Whether this message's sender is verified on some tier, with no failed verdict anywhere.
fn is_cryptographically_verified(req: &InjectionRequest, delivery_tier: &str) -> bool {
    let wan_instance_status = req.wan_instance.as_ref().map(|i| i.status);
    // Whether TIER=sensitive should actually STOP the receiving agent and
    // require human confirmation, vs. just carry the tag for visual
    // indication (SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md,
    // repo-owner-confirmed directly in a live conversation, same channel
    // this policy's own STOP rule already treats as authoritative).
    //
    // Cryptographic proof of identity is what the STOP rule guards the
    // absence of (a spoofed jekt followed by a spoofed "confirmation").
    // Once this message's sender is verified on some tier (host
    // `sig_verified`, reagent `reagent_verified` under the trusted key,
    // `lan_verified`, cross-channel `channel_verified`
    // (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D3), or `wan_verified`
    // from an `approved` or `new` install, W3-S §2.6), that attack is
    // ruled out, whatever made the message sensitive (a declared tier, or
    // a keyword in content a verified sender may legitimately discuss).
    //
    // Any failed verdict vetoes the relaxation (`any_failed`), so it never
    // covers an active-forgery case and another tier's success can't
    // cancel a failure. A failed `channel_verified` is not a forcing rule
    // above, but it still blocks the relaxation. A revoked install is
    // forcing, above; a WAN verdict with no instance relaxes nothing.
    // (see https://docs.agentmux.ai/internals/jekt-protocol/)
    let any_failed = [req.sig_verified, req.reagent_verified, req.lan_verified, req.channel_verified, req.wan_verified].contains(&Some(false));
    !any_failed
        && (req.sig_verified == Some(true)
        || req.reagent_verified == Some(true)
        || req.lan_verified == Some(true)
        || req.channel_verified == Some(true)
        || (delivery_tier == "wan"
            && req.wan_verified == Some(true)
            && matches!(
                wan_instance_status,
                Some(crate::backend::reactive::types::WanInstanceStatus::Approved | crate::backend::reactive::types::WanInstanceStatus::New)
            )))
}

impl Handler {
    /// Tidy the request before anything reads it, and return its request ID (made here if the
    /// caller gave none).
    pub(super) fn prepare_request(&mut self, req: &mut InjectionRequest) -> String {
        downgrade_untrusted_reagent(req);

        self.refuse_former_slug_sender(req); // never speaks, on any path (`former_slug.rs`)
        // Generate request ID if missing
        if req.request_id.is_none() || req.request_id.as_deref() == Some("") {
            req.request_id = Some(uuid::Uuid::new_v4().to_string());
        }
        req.request_id.clone().unwrap_or_default()
    }

    /// Resolve the target to its block, and the UID it resolved by, if any. A refusal is audited
    /// and returned as the response.
    pub(super) fn resolve_delivery_target(
        &mut self,
        req: &InjectionRequest,
        sanitized: &str,
        request_id: &str,
        now: u64,
        outcome_on_failure: Option<&str>,
    ) -> Result<(String, Option<String>), InjectionResponse> {
        // Resolve the target (identity M2, spec §4.4.3): UID first, then a
        // name held by exactly one live block. A name held by several is
        // refused WITH the candidates — the only place a collision is
        // visible is the only place it can be resolved (§5.2). The error
        // string deliberately does not begin with "agent not found": that
        // prefix is what `handle_reactive_inject` forwards to other
        // instances, and a local collision must not be forwarded.
        // The UID a target resolved by: its own, or the agent's whose
        // former slug it is. `None` for a name.
        Ok(match self.resolve_target(&req.target_agent) {
            TargetResolution::Uid(b) => (b, Some(req.target_agent.clone())),
            TargetResolution::FormerSlug { block, uid } => (block, Some(uid)),
            TargetResolution::Name(b) => (b, None),
            TargetResolution::Ambiguous(candidates) => {
                let err = format!(
                    "ambiguous agent name: '{}' is held by {} live agents — address one by uid: {}",
                    req.target_agent,
                    candidates.len(),
                    describe_candidates(&candidates)
                );
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    "",
                    sanitized,
                    false,
                    Some(&err),
                    request_id,
                    outcome_on_failure,
                    None,
                );
                return Err(early_refusal(request_id.to_string(), None, err, now));
            }
            TargetResolution::NotFound => {
                let err = format!("agent not found: {}", req.target_agent);
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    "",
                    sanitized,
                    false,
                    Some(&err),
                    request_id,
                    outcome_on_failure,
                    None,
                );
                return Err(early_refusal(request_id.to_string(), None, err, now));
            }
        })
    }

    /// Identity M2: a target that resolved by UID is confirmed by UID only.
    pub(super) fn confirm_uid_identity(
        &mut self,
        req: &InjectionRequest,
        sanitized: &str,
        request_id: &str,
        now: u64,
        block_id: &str,
        resolved_uid: Option<&str>,
    ) -> Result<(), InjectionResponse> {
        // Identity M2: a target that resolved by UID is confirmed by UID
        // only (`uid_identity_confirmer`); `None` is unverifiable and
        // delivery proceeds — see that field's doc comment for the
        // registered-but-unspawned case this protects. The name checks
        // below apply only to targets that resolved by name.
        if let Some(expected_uid) = resolved_uid {
            let own_uid = self
                .uid_identity_confirmer
                .as_ref()
                .and_then(|c| c(block_id));
            if let Some(own_uid) = own_uid {
                if !own_uid.eq_ignore_ascii_case(expected_uid) {
                    let err = format!(
                        "identity mismatch: block {} resolved for uid '{}' but its own uid is '{}'",
                        block_id, expected_uid, own_uid
                    );
                    tracing::error!(
                        target = %req.target_agent,
                        block_id = %block_id,
                        own_uid = %own_uid,
                        "reactive inject: recipient uid mismatch — rejecting delivery"
                    );
                    return Err(self.refuse_identity_mismatch(req, sanitized, request_id, now, block_id, err));
                }
            }
        }
        Ok(())
    }

    /// The recipient-identity check for a target that resolved by name.
    pub(super) fn confirm_name_identity(
        &mut self,
        req: &InjectionRequest,
        sanitized: &str,
        request_id: &str,
        now: u64,
        block_id: &str,
        resolved_by_uid: bool,
    ) -> Result<(), InjectionResponse> {
        // Recipient-identity check (issue #2695): before delivering, compare
        // the resolved block's own live, spawn-time-captured identity
        // (queried via `agent_identity_confirmer`, which goes straight to
        // the controller — independent of this handler's own agent_to_block
        // map) against who this jekt is addressed to. Checking agent_to_block
        // against itself would be a tautology and catch nothing; this is a
        // genuine second, independently-written source of truth that can
        // drift from the registry (e.g. a stale registry entry survived a
        // respawn, or a resync overwrote registration without updating the
        // block's own field).
        //
        // `None` (no confirmer configured, or this controller type doesn't
        // implement `agent_id()`) is "unverifiable," not "confirmed absent" —
        // delivery proceeds exactly as before this check existed. Matches
        // this codebase's existing jekt trust philosophy elsewhere (see the
        // TRUST=/ESCALATE= rules): only an ACTIVE mismatch is a red flag,
        // mere absence of proof is not.
        //
        // Checked against BOTH the live identity (`agent_identity_confirmer`)
        // and the stable one (`stable_agent_identity_confirmer`) — a target
        // resolved via the alias registry is legitimately the stable ID, not
        // the live display name, and would otherwise always fail this check
        // once the primary key has moved on post-rename. Either source
        // reporting a match is sufficient; both reporting `None` is
        // "unverifiable" exactly as before.
        if !resolved_by_uid
            && (self.agent_identity_confirmer.is_some()
                || self.stable_agent_identity_confirmer.is_some())
        {
            let live_agent_id = self
                .agent_identity_confirmer
                .as_ref()
                .and_then(|c| c(block_id));
            let stable_agent_id = self
                .stable_agent_identity_confirmer
                .as_ref()
                .and_then(|c| c(block_id));
            // Both `None` is "unverifiable" (see above) — only proceed to
            // the check below if at least one source reported an identity.
            if live_agent_id.is_some() || stable_agent_id.is_some() {
                let target_lower = req.target_agent.to_lowercase();
                let matches = |id: &Option<String>| {
                    id.as_deref().is_some_and(|id| id.to_lowercase() == target_lower)
                };
                if !matches(&live_agent_id) && !matches(&stable_agent_id) {
                    let err = format!(
                        "identity mismatch: block {} resolved for target '{}' but its own identity is '{}'",
                        block_id,
                        req.target_agent,
                        live_agent_id
                            .as_deref()
                            .or(stable_agent_id.as_deref())
                            .unwrap_or("<unknown>")
                    );
                    tracing::error!(
                        target = %req.target_agent,
                        block_id = %block_id,
                        live_agent_id = ?live_agent_id,
                        stable_agent_id = ?stable_agent_id,
                        "reactive inject: recipient identity mismatch — rejecting delivery"
                    );
                    return Err(self.refuse_identity_mismatch(req, sanitized, request_id, now, block_id, err));
                }
            }
        }
        Ok(())
    }

    /// Controller-aware delivery. `Some(response)` ends the delivery; `None` falls through to
    /// keystroke injection.
    pub(super) fn deliver_structured(&mut self, d: &Delivery<'_>, final_msg: &str) -> Option<InjectionResponse> {
        // Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3).
        // Persistent (stream-json) and ACP agents have no PTY — their inbox is a
        // structured channel (live stdin NDJSON / `session/prompt`). Delivering there
        // also lands the message mid-turn (steering) instead of waiting for idle.
        // PTY-based shell/term agents report back so we fall through to keystrokes.
        if let Some(ref deliver) = self.message_sender {
            let joins_turn = crate::backend::reactive::turn_audit::turn_in_flight(&d.block_id);
            match deliver(&d.block_id, final_msg, d.req.files_delivery.as_ref().map_or(&[][..], |f| &f.inline), &crate::backend::reactive::sanitize::jekt_origin(d.req, d.effective_tier, d.delivery_tier, &d.request_id, d.now as i64), d.req.expires_at_ms) {
                Ok(outcome @ (SenderDelivery::Delivered | SenderDelivery::Deferred)) => {
                    let deferred = outcome == SenderDelivery::Deferred;
                    self.delivery_turn = Some(if joins_turn && !deferred { "joined" } else { "started" });
                    tracing::info!(
                        target_agent = %d.req.target_agent,
                        block_id = %d.block_id,
                        deferred,
                        "inject: structured delivery to non-PTY controller"
                    );
                    self.audit_delivery(d, None);
                    return Some(d.response(None, Some(deferred)));
                }
                Ok(SenderDelivery::Pty) => {
                    // PTY-based controller — fall through to keystroke injection.
                }
                Err(e) => {
                    // Structured controller but delivery failed (e.g. persistent
                    // process not running). Do NOT fall back to PTY keystrokes — the
                    // persistent controller rejects raw input. Surface the error.
                    tracing::warn!(
                        target_agent = %d.req.target_agent,
                        block_id = %d.block_id,
                        error = %e,
                        "inject: structured delivery failed"
                    );
                    self.audit_delivery(d, Some(&e));
                    return Some(d.response(Some(e), None));
                }
            }
        }
        None
    }

    /// Deliver as keystrokes through the input sender, then audit the outcome.
    pub(super) fn deliver_to_pty(&mut self, d: &Delivery<'_>, final_msg: &str) -> InjectionResponse {
        // Send message via input sender
        let sender = match &self.input_sender {
            Some(s) => s.clone(),
            None => {
                let err = "input sender not configured".to_string();
                self.audit_delivery(d, Some(&err));
                return d.response(Some(err), None);
            }
        };

        // Jekt inject sequence (see docs/specs/jekt-inject-timing.md):
        // 1. \r to clear any partial input on the line
        // 2. message\r as single payload (proven to display text — v0.31.122/125)
        // 3. Three delayed \r at 200ms intervals as separate PTY writes to submit
        let _ = sender(&d.block_id, b"\r");
        let payload = format!("{}\r", final_msg);
        tracing::info!(
            target_agent = %d.req.target_agent,
            block_id = %d.block_id,
            msg_len = payload.len(),
            "inject: sending payload to PTY"
        );
        if let Err(e) = sender(&d.block_id, payload.as_bytes()) {
            tracing::error!(
                target_agent = %d.req.target_agent,
                block_id = %d.block_id,
                error = %e,
                "inject: sender failed"
            );
            self.audit_delivery(d, Some(&e));
            return d.response(Some(e), None);
        }

        // Spawn 3 delayed \r sends as separate PTY events to ensure submission.
        let sender_enter = sender.clone();
        let block_id_enter = d.block_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
        });

        // Success
        self.audit_delivery(d, None);
        d.response(None, None)
    }

    /// Audit and answer a recipient-identity mismatch.
    fn refuse_identity_mismatch(
        &mut self,
        req: &InjectionRequest,
        sanitized: &str,
        request_id: &str,
        now: u64,
        block_id: &str,
        err: String,
    ) -> InjectionResponse {
        self.log_audit(
            req.source_agent.as_deref(),
            &req.target_agent,
            block_id,
            sanitized,
            false,
            Some(&err),
            request_id,
            Some("identity-mismatch"),
            None,
        );
        early_refusal(request_id.to_string(), Some(block_id.to_string()), err, now)
    }

    /// Write the audit entry for a delivery that reached its target.
    fn audit_delivery(&mut self, d: &Delivery<'_>, error: Option<&str>) {
        let (success, outcome) = if error.is_none() { (true, d.outcome_on_success) } else { (false, d.outcome_on_failure) };
        self.log_audit(
            d.req.source_agent.as_deref(),
            &d.req.target_agent,
            &d.block_id,
            d.sanitized,
            success,
            error,
            &d.request_id,
            outcome,
            None,
        );
    }
}
