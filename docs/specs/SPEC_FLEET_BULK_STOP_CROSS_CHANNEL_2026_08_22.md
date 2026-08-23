# SPEC: `FleetBulkStop` reaches cross-channel targets; LAN/WAN deliberately deferred

**Date:** 2026-08-22
**Status:** Implemented (cross-channel only)
**Author:** Korp
**Repo touched:** `agentmux` (`agentmux-srv/src/server/app_api/fleet.rs`, `server/mod.rs`, `server/app_api/mod.rs`)
**Diagnosis:** `docs/reports/REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md` §3.2
**Related:** `SPEC_FLEET_BROADCAST_CROSS_TIER_TARGETING_2026_08_22.md` (the sibling fix this follows)

## 1. Problem

`fleet_bulk_stop_impl` resolved every target purely via
`state.reactive_handler.get_agent_by_block` — this instance's own
in-process controller registry. A `host.cross_channel` target (a different
channel on the same host, visible via `FleetList`/`DiscoverAgents`, with a
real `block_id` in the same namespace) always failed with "no registered
agent for this block," identical to a genuinely-unknown block_id — even
though the block is live, just owned by a sibling process on the same
machine.

## 2. Fix — cross-channel only

Unlike `FleetBroadcast`, bulk-stop involves **no jekt signing at all**
(same module doc comment, unchanged) — so there's no signing-locality
reason to keep the fix client-side. This is fixed once, at the single
shared choke point (`fleet_bulk_stop_impl`), covering BOTH the WS-RPC
(Swarm UI) and HTTP (`FleetBulkStop` MCP tool) callers identically.

- When a target isn't found in the local controller registry, look it up
  in the shared cross-channel registry (`registry::list_all_shared` — the
  same mechanism `server/reactive.rs`'s inject cascade tier-2b already
  uses) by `block_id`.
- If found, and the entry is loopback (same defense-in-depth check the
  inject cascade already applies) and isn't a stale self-entry, forward an
  authenticated HTTP stop request to a new endpoint,
  `POST /agentmux/agent/stop`, on that channel's own srv — using ITS OWN
  `auth_key` from the registry entry, not this instance's. Same one-hop,
  same-host, same-user trust model as the existing inject forward; no new
  trust primitive.
- If the shared registry has no matching entry either, falls through to
  the original `stop_one_agent_block` call — same exact "NOT_RUNNING"
  error a genuinely-unknown block_id always produced. No regression for
  the common case.
- `fleet_bulk_stop_impl` is now `async` (previously synchronous) to allow
  the forward's HTTP round-trip; both call sites (`register_fleet_bulk_stop`
  WS-RPC handler, `handle_fleet_bulk_stop` HTTP handler) now `.await` it.
  `app_api::agent_io` module visibility widened to `pub(crate)` so the new
  `/agentmux/agent/stop` handler (in `server/mod.rs`) can call
  `stop_one_agent_block` directly — the exact same function the local path
  already used, reused rather than duplicated for the forwarded case.

## 3. LAN/WAN — deliberately deferred, not silently dropped

Extending bulk-stop past cross-channel needs a genuinely new primitive:
per `REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md` §2,
**no remote-command bus exists on any tier today** — jekt is
message-only (free-text injection into a conversation stream), never a
structured "run this" verb. A LAN/WAN remote-stop would mean designing and
shipping a brand-new authenticated destructive-action protocol, not
reusing something that already exists the way the cross-channel fix does.

Two additional reasons this isn't attempted here:

- **LAN** could plausibly reuse the LAN Ed25519 signing infrastructure
  (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`) for a new `stop_request`/
  `stop_response` message pair — architecturally sound, but a genuinely new
  message type with its own tier/escalation semantics deserves its own
  dedicated design pass (e.g.: should a cryptographically-verified LAN
  sender's stop request skip human escalation the way `ESCALATE=none`
  already works for messaging, or does a destructive remote action always
  warrant `ESCALATE=required` regardless of verification? — not a decision
  to fold into this PR's scope).
- **WAN** is currently blocked on the exact same infrastructure gap as
  general WAN agent-to-agent jekt signing (audit §4.1): no non-reagent WAN
  sender is cryptographically verifiable today, and that's real
  `agentmux-cloud`/`shared-infrastructure` work (Cognito M2M redesign), not
  something buildable from this repo. Building WAN bulk-stop without that
  would mean either trusting an unverified WAN sender with a destructive
  action (precisely the kind of gap this whole audit exists to close, not
  add to) or inventing a parallel, one-off WAN trust mechanism that
  duplicates the already-planned redesign.

Both are flagged as real, visible follow-ups — not silently assumed
covered by this PR's title.

## 4. Testing

- `forward_stop_to_shared_channel`: 4 new unit tests — returns `None` for
  no matching entry; returns `None` for a non-loopback entry (security);
  returns `None` for a stale self-entry; succeeds against a real loopback
  peer (spawned via a raw-TCP fake responder, mirroring the existing
  `spawn_fake_browser_api` pattern) and asserts the PEER's own `auth_key`
  is what gets sent, not this instance's; propagates a peer-side failure
  response's error text.
- `fleet_bulk_stop_impl`: 1 new end-to-end test — a target present only in
  the shared cross-channel registry (absent from this instance's own)
  succeeds via the forward, not "NOT_RUNNING."
- Full `agentmux-srv` suite: 2721/2722 passed (the one failure,
  `refresh_processes_specifics_does_not_leak_a_handle_per_call`, is an
  unrelated, environment-sensitive Windows handle-count check — confirmed
  to pass in isolation, reproduces the same way with or without this
  change, and touches no code this PR modifies).
