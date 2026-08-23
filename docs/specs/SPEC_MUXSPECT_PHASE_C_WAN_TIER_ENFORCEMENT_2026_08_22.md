# SPEC: `muxspect` Phase C — WAN tier enforcement

**Date:** 2026-08-22
**Status:** Implemented (scoped — see §2 for why this is small)
**Author:** Korp
**Repo touched:** `agentmux` (`agentmux-srv`)
**Related:** `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
(Phase C's design), `docs/specs/SPEC_MUXSPECT_PHASE_B_POLICY_AND_TIER_ENFORCEMENT_2026_08_22.md`
(Phase B, this extends)

## 1. The one real gap Phase B left for WAN

Phase B's `resolve_transcript_request_tier_fields` (`server/reactive.rs`)
was written tier-generically from the start — it reads `req.delivery_tier`
and matches trust grants against whichever tier a request actually arrived
on, never assuming "lan." So the RULE LOGIC already worked for `"wan"`
without any changes.

But it was only ever **called** from `handle_reactive_inject` — the HTTP
handler behind `/agentmux/reactive/inject`. WAN delivery doesn't go
through that path at all: `muxbus::cloud_subscriber::sync_agent_reactive`
polls the cloud relay for pending injections and calls
`Handler::inject_message` **directly**, bypassing
`server/reactive.rs`/HTTP entirely. Without also calling the resolution
function there, a WAN-delivered `transcript_request` would silently skip
both confirmed jekt rules completely — not a weaker version of them,
nothing at all.

## 2. Fix

- `resolve_transcript_request_tier_fields` made `pub(crate)`, its
  parameter changed from `&AppState` to `&Arc<Store>` directly (all it ever
  used from `AppState` was `.wstore`) — `sync_agent_reactive` has its own
  `wstore: &Arc<Store>` parameter already, no new plumbing needed.
- `server::reactive` module widened to `pub(crate)` so
  `muxbus::cloud_subscriber` (a sibling top-level module) can reach it.
- One new call, right before `sync_agent_reactive`'s existing
  `handler.inject_message(req)`.

That's the entire change. Everything else — the `TranscriptRequest`/
`TranscriptResponse` wire payload, the `conversation_visibility` setting,
`db_conversation_trust_grants`, the actual rule 1/rule 2 computation — is
unchanged from Phase B and now correctly reachable from both delivery
paths.

## 3. Why this is small — the design spec's own WAN-specific items are all N/A yet

`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`'s Phase C
section lists three WAN-specific differences from Phase B. All three
concern pieces Phase B explicitly deferred (see that spec's §3) and this
PR doesn't build either — so there is currently nothing to differentiate:

- *"`conversation_visibility` for WAN defaults to `private`, and
  `trusted_peers` shouldn't be exposed as usable for WAN"* — this is
  guidance for a settings UI/CLI that doesn't exist yet (nothing lets an
  agent SET `conversation_visibility` or manage trust grants today). Moot
  until that surface is built. The underlying data model already supports
  the recommendation once it matters: a `trusted_peers` grant is tier-
  scoped (`db_conversation_trust_grants.tier`), so a WAN grant only ever
  relaxes escalation for an ACTUAL WAN request — nothing silently
  leaks across tiers regardless of what any future settings UI allows.
- *"`ask` mode requests no session-remembered/cached approval across
  WAN"* — about the (not-yet-built) approve/deny CLI's caching behavior.
  Moot until that's built.
- *"Default `max_lines` for WAN needs bench data"* — about the
  (not-yet-built) auto-responder's transcript-reading logic. Moot until
  that's built.

None of this is silently glossed over — it's the same non-goal list Phase
B already named, now confirmed to also cover Phase C's own described
scope, not just Phase B's.

## 4. Testing

- 2 new tests in `server::reactive::transcript_request_tier_resolution_tests`
  proving the shared resolution function behaves identically for
  `delivery_tier: "wan"` as it already does for `"lan"`: rule 1 forces
  sensitive; a `trusted_peers` grant checked against a MATCHING `"wan"`
  request relaxes escalation (the positive case — the existing Phase B
  suite already covered the negative "wrong tier" case, granted-for-WAN-
  but-requested-on-LAN).
- Full `agentmux-srv` suite: 2746 passed, 0 failed, 6 pre-existing ignores.
- `muxbus::cloud_subscriber::sync_agent_reactive` itself is not
  independently integration-tested — consistent with this file's own
  existing convention (every test in `cloud_subscriber.rs` covers an
  extracted pure helper — `is_credential_rejected`, `reagent_sig_is_fresh`,
  etc. — never the top-level async polling/delivery orchestration, which
  needs real HTTP mocking this file has never built). The correctness of
  what gets called is covered by the resolution function's own thorough
  test suite; the one-line call site is a small, visually-verifiable
  addition consistent with that existing testing boundary, not a new gap.
