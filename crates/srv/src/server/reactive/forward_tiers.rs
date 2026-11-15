// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The forwarding tiers of [`deliver`](super::deliver), for a target this instance does not
//! run: a sidecar on this host, another channel's entry in the shared registry, a LAN
//! peer, then the cloud relay or a hold. Moved out of `deliver` without a change in behaviour.

use super::*;
use crate::backend::message_flow::{MessageFlowOutcome, MessageFlowVia};
use crate::backend::reactive::types::InjectionResponse;

/// What the tiers share while one jekt looks for its target.
pub(super) struct Tiers<'a> {
    state: &'a AppState,
    auth_via: crate::server::ReactiveAuthVia,
    caller_uid: &'a str,
    req: &'a InjectionRequest,
    resp: &'a InjectionResponse,
    forwarded_req: InjectionRequest,
    same_host_req: InjectionRequest,
    same_host_tier: &'a str,
    /// Whether any forwarding tier found the target alive elsewhere. A target that is alive but
    /// refused is not "absent" and is never held here (durable jekt spec §2.1, condition 3); a
    /// same-host entry whose process is dead — every entry after an srv restart, which changes its
    /// port — is not a candidate, so the first message after a restart is held.
    candidate_seen: bool,
    /// A candidate an older build registered can't take files (spec §3.4).
    files_too_old: bool,
    /// A former name's agent, or a display name's, runs as its slug (`former_slug`).
    alias: Option<String>,
    /// The LAN peer was sent the message and gave no answer in time.
    silent_peer: bool,
}

impl<'a> Tiers<'a> {
    pub(super) fn new(
        state: &'a AppState,
        auth_via: crate::server::ReactiveAuthVia,
        caller_uid: &'a str,
        req: &'a InjectionRequest,
        resp: &'a InjectionResponse,
    ) -> Self {
        // Every forward below sends this hop-incremented request, not the
        // original `req` — a peer that also fails to find the agent locally
        // and forwards onward needs to see the accumulated hop count too.
        let mut forwarded_req = req.clone();
        forwarded_req.forward_hops = req.forward_hops.saturating_add(1);

        // SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D4: a same-host hop to a
        // DIFFERENT instance (Tier 2a/2b below) is labelled `channel`, not
        // `host` — `host` is reserved for genuinely same-instance traffic, so
        // the label finally matches the guarantee (the receiving instance holds
        // no HMAC key for a sender it didn't spawn; §2.3 of the spec). The
        // receiver can't tell a forward from a local call by auth alone (the
        // forward authenticates with the receiver's own full `auth_key`), so
        // the forwarding side says so here, and `resolve_delivery_tier` on the
        // far side honours a full-key caller's claim as it always has.
        //
        // An already-`lan`/`wan` hop keeps its label: that's what lets the peer
        // re-derive its own tier-specific verification on the second hop
        // (`resolve_delivery_tier`'s doc comment, reagentx P0 on the LAN signing
        // PR). A `channel` hop forwarded again stays `channel`. Tier 3 (LAN)
        // sends `forwarded_req` untouched — the peer authenticates it via
        // `lan_key` and forces `lan` regardless of the body.
        let same_host_tier = same_host_forward_tier(req.delivery_tier.as_deref());
        let mut same_host_req =
            lan_v2::channel_sign_at_origin(&state.mstore, caller_uid, forwarded_req.clone());
        same_host_req.delivery_tier = Some(same_host_tier.to_string());
        let alias = former_slug::delivery_alias(state, &req.target_agent);
        Self {
            state,
            auth_via,
            caller_uid,
            req,
            resp,
            forwarded_req,
            same_host_req,
            same_host_tier,
            candidate_seen: false,
            files_too_old: false,
            alias,
            silent_peer: false,
        }
    }

    /// Tries each tier in turn; `Some` is the answer to send back. `None`: every tier
    /// declined, so the caller returns the local error.
    pub(super) async fn run(
        mut self,
        replay: &mut replay::Reservation,
    ) -> Option<serde_json::Value> {
        if let Some(body) = self.same_host_sidecar().await {
            self.note_flow(MessageFlowVia::Channel, &body, None);
            return Some(body);
        }
        if let Some(body) = self.other_channels().await {
            self.note_flow(MessageFlowVia::Channel, &body, None);
            return Some(body);
        }
        if let Some(body) = self.lan_peer().await {
            let host = flow_notes::lan_host_of(self.state, &self.req.target_agent);
            self.note_flow(MessageFlowVia::Lan, &body, host.as_deref());
            return Some(body);
        }
        let body = self.relay_or_hold(replay).await?;
        // A queued message was reported where the relay took it; what is left here is held or refused.
        if flow_notes::answer_outcome(&body) != MessageFlowOutcome::Delivered {
            self.note_flow(flow_notes::arrival_via(self.req), &body, None);
        }
        Some(body)
    }

    /// Tells the Atlas map what `body`, the answer of a forwarding tier, made of the message.
    fn note_flow(&self, via: MessageFlowVia, body: &serde_json::Value, to_host: Option<&str>) {
        flow_notes::note(self.state, self.req, via, flow_notes::answer_outcome(body), to_host);
    }

    /// Tier 2a: same host, a different sidecar of this channel (file registry → HTTP loopback).
    async fn same_host_sidecar(&mut self) -> Option<serde_json::Value> {
        let (state, req) = (self.state, self.req);
        let data_dir = base::get_mux_data_dir();
        if let Some((found_as, entry)) =
            former_slug::registry_entry(&data_dir, &req.target_agent, self.alias.as_deref())
        {
            // Guard against self-forwarding loops.
            if entry.local_url != state.local_web_url
                && crate::server::jekt_files::entry_takes(req, &entry, &mut self.files_too_old)
            {
                match forward_inject_to_peer(
                    state,
                    req,
                    &self.same_host_req,
                    ForwardPeer {
                        url: &entry.local_url,
                        auth_key: &entry.auth_key,
                        kind: "cross-instance",
                        channel: None,
                        trust: EchoTrust {
                            delivery_tier: self.same_host_tier,
                            sig_verified: req.sig_verified,
                            reagent_verified: req.reagent_verified,
                            lan_verified: req.lan_verified,
                            channel_verified: req.channel_verified,
                        },
                    },
                )
                .await
                {
                    ForwardOutcome::Delivered(body) | ForwardOutcome::Refused(body) => {
                        return Some(body)
                    }
                    // A non-delivery here is ambiguous: this entry may be
                    // stale (agent unregistered without a clean shutdown), OR
                    // the owning process may be alive and simply not have
                    // registered this specific agent yet — e.g. right after
                    // that channel's srv came up. Connection-level failure is
                    // at least as plausible a transient trigger as a parsed
                    // success:false body, which is why both land here.
                    // `should_evict_on_forward_failure` combines PID-liveness
                    // with the entry's age: a dead process always evicts; a
                    // live process only protects a FRESH entry (the actual
                    // startup race), not an old one — an old entry whose
                    // process happens to still be alive for OTHER agents is
                    // presumed to be its own genuinely-dead agent (reagent P1
                    // round 2 on #2640: PID alone over-protects, since it
                    // identifies the whole srv process, not this one agent).
                    // See docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                    //
                    // Falls through to Tier 2b/3 either way (reagent P1 on
                    // #2350 — this previously returned unconditionally
                    // whenever the body parsed, regardless of success, so a
                    // stale same-channel entry never reached a later tier).
                    ForwardOutcome::Stale | ForwardOutcome::NoAnswer => {
                        if former_slug::may_evict(&found_as, req)
                            && agent_registry::should_evict_on_forward_failure(&entry)
                        {
                            tracing::warn!(
                                target = %req.target_agent,
                                pid = entry.pid,
                                "cross-instance forward: entry presumed dead — evicting and falling through"
                            );
                            agent_registry::remove(&data_dir, &found_as);
                        } else {
                            // Alive but did not take it: the target exists
                            // elsewhere, so it is not held here.
                            self.candidate_seen = true;
                            tracing::warn!(
                                target = %req.target_agent,
                                pid = entry.pid,
                                "cross-instance forward: entry is fresh and owning process is alive — NOT evicting, falling through"
                            );
                        }
                    }
                    // Says nothing about this entry's liveness — leave it be,
                    // and do not treat the target as absent.
                    ForwardOutcome::Inconclusive => self.candidate_seen = true,
                }
            }
        }
        None
    }

    async fn other_channels(&mut self) -> Option<serde_json::Value> {
        let (state, req) = (self.state, self.req);
        // Tier 2b: same host, DIFFERENT channel (host-global shared registry).
        // Runs when Tier 2a had no same-channel entry or its forward already
        // failed above — closes the gap issue #1916 tracked (Tier 2 previously
        // only ever reached agents in the caller's own channel). Candidates are
        // tried freshest-first (§4.3 of the cross-channel delivery spec);
        // a failed forward evicts just that channel's entry and falls through
        // to the next candidate, same evict-on-fail shape Tier 3 already uses.
        if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
            let (found_as, candidates) =
                former_slug::shared_entries(&shared_dir, &req.target_agent, self.alias.as_deref());
            for entry in candidates {
                // Self-forward guard, matching Tier 2a. Also loopback-only
                // (§5 of the spec): a poisoned registry entry can't redirect
                // a forward off-box, since resolve_shared_reactive_dir() is a
                // same-user local file, but defense in depth costs nothing here.
                let is_loopback = entry.local_url.starts_with("http://127.0.0.1")
                    || entry.local_url.starts_with("http://localhost")
                    || entry.local_url.starts_with("http://[::1]");
                if !is_loopback
                    || entry.local_url == state.local_web_url
                    || !crate::server::jekt_files::entry_takes(req, &entry, &mut self.files_too_old)
                {
                    continue;
                }

                match forward_inject_to_peer(
                    state,
                    req,
                    &self.same_host_req,
                    ForwardPeer {
                        url: &entry.local_url,
                        auth_key: &entry.auth_key,
                        kind: "cross-channel",
                        channel: Some(&entry.channel),
                        trust: EchoTrust {
                            delivery_tier: self.same_host_tier,
                            sig_verified: req.sig_verified,
                            reagent_verified: req.reagent_verified,
                            lan_verified: req.lan_verified,
                            channel_verified: req.channel_verified,
                        },
                    },
                )
                .await
                {
                    ForwardOutcome::Delivered(body) | ForwardOutcome::Refused(body) => {
                        return Some(body)
                    }
                    // Same ambiguity and same should_evict_on_forward_failure
                    // policy as Tier 2a — PID-liveness alone over-protects an
                    // old, genuinely-dead individual agent whose srv process
                    // another agent keeps alive (reagent P1 round 2 on #2640),
                    // and a connection failure can be the same startup-race
                    // transient (target channel's srv not listening yet). See
                    // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
                    // Evicting only this channel's entry lets the loop try the
                    // next candidate.
                    ForwardOutcome::Stale | ForwardOutcome::NoAnswer => {
                        if former_slug::may_evict(&found_as, req)
                            && agent_registry::should_evict_on_forward_failure(&entry)
                        {
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                pid = entry.pid,
                                "cross-channel forward: entry presumed dead — evicting and trying next candidate"
                            );
                            agent_registry::remove_shared(&shared_dir, &found_as, &entry.channel);
                        } else {
                            self.candidate_seen = true;
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                pid = entry.pid,
                                "cross-channel forward: entry is fresh and owning process is alive — NOT evicting, trying next candidate"
                            );
                        }
                    }
                    ForwardOutcome::Inconclusive => self.candidate_seen = true,
                }
            }
        }
        None
    }

    async fn lan_peer(&mut self) -> Option<serde_json::Value> {
        let (state, req, caller_uid) = (self.state, self.req, self.caller_uid);
        // Tier 3: LAN peer (mDNS lookup → HTTP). Runs when tier 2 had no registry
        // entry or its forward failed. Queries each discovered LAN peer for the
        // agent; result is cached for 60s to avoid per-inject mDNS fan-out.
        let lan_peer = match state
            .lan_discovery
            .find_agent(&req.target_agent, &state.http_client)
            .await
        {
            Some((url, key)) => match crate::server::jekt_files::lan_peer_takes(
                &state.http_client,
                req,
                &url,
                &key,
                &mut self.files_too_old,
                &mut self.candidate_seen,
            )
            .await
            {
                Ok(takes) => takes.then_some((url, key)),
                Err(refusal) => return Some(refusal),
            },
            None => None,
        };
        if let Some((peer_url, peer_auth_key)) = lan_peer {
            let lan_forwarded = lan_v2::sign_and_log(
                &state.mstore,
                caller_uid,
                self.forwarded_req.clone(),
                &peer_url,
            );
            let outcome = forward_inject_to_peer(
                state,
                req,
                &lan_forwarded,
                ForwardPeer {
                    url: &peer_url,
                    auth_key: &peer_auth_key,
                    kind: "lan-peer",
                    channel: None,
                    trust: EchoTrust {
                        delivery_tier: "lan",
                        sig_verified: None,
                        // reagent signing is WAN-only; it never applies on the
                        // LAN forward path.
                        reagent_verified: None,
                        lan_verified: req.lan_verified,
                        channel_verified: req.channel_verified,
                    },
                },
            )
            .await;
            self.silent_peer = matches!(outcome, ForwardOutcome::NoAnswer);
            match outcome {
                ForwardOutcome::Delivered(body) | ForwardOutcome::Refused(body) => {
                    return Some(body)
                }
                // The peer answered "not mine" (agent migrated away since we
                // cached it) or was unreachable — either way the discovery
                // cache entry is wrong. Unlike the registry tiers there is no
                // freshness/PID guard to weigh: the cache is cheap to refill
                // from mDNS, so evicting on any non-delivery is the right
                // trade rather than an oversight.
                // A stale route is evicted as wrong, so it is not a live
                // candidate (Codex P1 on #3632): the message may still be
                // held. Only an inconclusive answer leaves the target
                // possibly alive there.
                ForwardOutcome::Stale | ForwardOutcome::NoAnswer => {
                    state.lan_discovery.evict_agent(&req.target_agent);
                }
                ForwardOutcome::Inconclusive => self.candidate_seen = true,
            }
        }
        None
    }

    async fn relay_or_hold(
        &mut self,
        replay: &mut replay::Reservation,
    ) -> Option<serde_json::Value> {
        let (state, req, auth_via, caller_uid, resp) = (
            self.state,
            self.req,
            self.auth_via,
            self.caller_uid,
            self.resp,
        );
        // Tier 4: cloud relay (muxbus store-and-forward). Runs when no local,
        // same-host or LAN tier could place the message.
        //
        // This tier used to be a comment here delegating to a "muxbus-client"
        // that the callers agents actually use are not — the MCP `Message`
        // tool's send action POSTs to this very endpoint and bails on `success != true`, so
        // the chain silently stopped at three while that tool's description
        // promised "local → LAN → cloud". Owning it here means every caller
        // inherits it. See `crate::muxbus::relay` and
        // REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md §5.
        //
        // Except for an agent defined here that no tier found live: that one
        // is held here unless the relay says another install of the account
        // runs it (PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md §5). Before
        // this, the relay came first for it too, so a same-computer agent got
        // the relay's 30 min expiry instead of the 24 h hold, and only a
        // sender NOT signed in to the cloud ever reached the hold.
        if req.files.is_some() {
            return Some(
                crate::server::jekt_files::no_route(
                    state,
                    auth_via,
                    caller_uid,
                    req,
                    resp,
                    self.candidate_seen,
                    self.files_too_old,
                    &mut *replay,
                )
                .await,
            );
        }
        // A peer that answers its polls but not this message: kept here and sent again by the replay
        // (`peer_hold`), not left to the relay's 30 minutes (task #2512).
        if self.silent_peer && !self.candidate_seen {
            if let Some(held) = peer_hold::hold_for_silent_peer(state, auth_via, req, resp).await {
                return Some(held);
            }
        }
        if !self.candidate_seen
            && route_for_absent(state, auth_via, req, resp).await == AbsentRoute::HoldHere
        {
            if let Some(held) = hold_for_absent_target(
                state,
                auth_via,
                req,
                resp,
                HoldReason::NotRunning,
                &mut *replay,
            )
            .await
            {
                return Some(held);
            }
        }
        // An agent of a LAN peer that is down: kept here for 24 h, not left to the relay's 30 minutes
        // (`peer_hold`).
        if !self.candidate_seen {
            if let Some(held) = peer_hold::hold_for_down_peer(state, auth_via, req, resp).await {
                return Some(held);
            }
        }
        if let Some(body) = try_cloud_relay(state, req, caller_uid).await {
            return Some(body);
        }
        // 5. No tier knew the target: hold it for the agent's return, when
        //    the durable jekt conditions hold.
        if !self.candidate_seen {
            if let Some(held) = hold_for_absent_target(
                state,
                auth_via,
                req,
                resp,
                HoldReason::NotRunning,
                &mut *replay,
            )
            .await
            {
                return Some(held);
            }
        }
        None
    }
}
