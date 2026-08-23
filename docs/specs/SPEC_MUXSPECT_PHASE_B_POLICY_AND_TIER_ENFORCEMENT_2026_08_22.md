# SPEC: `muxspect` Phase B — policy infrastructure + jekt tier enforcement

**Date:** 2026-08-22
**Status:** Implemented (scoped — see §3 for what's deferred)
**Author:** Korp
**Repo touched:** `agentmux` (`agentmux-srv`, `agentmux-common`)
**Related:** `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
(the design this implements a slice of), `docs/specs/SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`
(the confirmed jekt rules this enforces)

## 1. What shipped

The confirmed jekt rules for `transcript_request` need real infrastructure
to mean anything: a place to store each agent's disclosure policy, a wire
format for the request/response pair, and the actual escalation-computation
wiring. This PR ships all three, fully tested, plus the trust-grant table
Phase B's `trusted_peers` mode needs — everything **except** the pieces
that build ON TOP of correct tiering (see §3).

### 1.1 `conversation_visibility` (schema v26)

New column on `db_agent_definitions` (+ dual-written to `db_agents`,
mirroring `auto_continue_enabled`'s treatment — a simple per-agent
opt-in-style setting). One of `"private"` (default, fail-closed) /
`"trusted_peers"` / `"ask"`. Channel-local only, like
`model_vendor_base_url`/`memory_id` — deliberately not added to
`DefinitionRecordV1`'s cross-channel wire format; a cross-channel-reopened
agent starts back at the safe `"private"` default rather than carrying a
stale value (`def_registry_mirror.rs`, `agent_def_list()`'s own overlay
preserves the local row's real value when one exists, same precedent as
those two fields).

### 1.2 `db_conversation_trust_grants`

New table, mirroring `db_lan_peer_pubkey_pins`'s exact shape (own module
`conversation_trust_grants.rs`, get/set-style methods, case-insensitive
agent_id lookups) — the explicit precedent
`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md` named.
`(agent_id, granted_peer_agent_id, tier)` — a grant is tier-scoped: one
tier's cryptographic identity guarantee is never assumed to cover another.

### 1.3 Wire payload (`agentmux-common::transcript_request`)

`TranscriptRequest`/`TranscriptResponse`, JSON-encoded and carried AS a
jekt's existing `message: String` field — there is no structured
`msg_type` anywhere in the jekt wire format (`message` is part of the
signed material every signer/verifier already depends on; adding a
top-level field would mean touching all of them for a feature that's
otherwise fully layerable on top). `parse_transcript_request`/
`parse_transcript_response` sniff for the shape and return `None` for
anything else — malformed JSON, unrelated JSON, or ordinary free text —
so this can never misclassify normal jekt traffic.

### 1.4 Escalation-rule enforcement (the security-critical core)

Two new server-computed fields on `InjectionRequest`
(`is_transcript_request`, `transcript_request_escalate_forced`) —
`#[serde(skip_deserializing)]`, same guarantee as `sig_verified`/
`lan_verified`: no attacker-supplied JSON body can set or suppress them.

- `resolve_transcript_request_tier_fields` (`server/reactive.rs`) runs in
  `handle_reactive_inject`, right after signature verification, before the
  request reaches `Handler::inject_message` (which has no `Store` access
  "by design" — same reason `sig_verified`/`lan_verified` are resolved by
  this same caller). Re-parses `message` itself; looks up the RESPONDING
  agent (`target_agent`) by **slug**, not display name (the same
  cross-namespace hazard this file's own Supervisor-nudge opt-in check
  already guards against, mirrored exactly); for `trusted_peers`, checks
  `db_conversation_trust_grants` against the actual delivery tier.
- `Handler::inject_message_inner` (`backend/reactive/handler.rs`):
  `is_transcript_request` ORs into the existing `is_sensitive` computation
  (rule 1 — unconditional force, any tier, any trust); `requires_stop`
  becomes `is_sensitive && (!is_cryptographically_verified ||
  transcript_request_escalate_forced)` (rule 2 — the one named exception to
  the 2026-08-17 verified-sender relaxation).

## 2. Design decisions worth calling out

- **Fail-closed on every branch.** An unknown target agent, a agent
  definition load error, or an unrecognized `conversation_visibility`
  value all resolve `transcript_request_escalate_forced` to whatever is
  SAFER — for the escalate-forcing question specifically, "private-like"
  (no forcing beyond the ordinary rule) is the correct fail-closed
  behavior, because rule 1's forced `TIER=sensitive` already applies
  regardless and a genuinely "private" or not-yet-configured agent was
  never going to auto-disclose anything once §3's auto-responder exists —
  there's no scenario where failing this lookup open would leak content.
- **Layering, not a new transport.** No new HTTP endpoint, no new signing
  scheme — a `transcript_request` rides the exact same
  `/agentmux/reactive/inject` path, signing, and cross-tier delivery
  cascade every ordinary jekt already uses. The only new logic is
  detection + tier computation.

## 3. Explicitly deferred — not silently incomplete

**The "invisible auto-resolve" convenience layer is not built.** Per the
design spec, `private` should auto-deny and `trusted_peers` (when granted)
should auto-approve **without the target agent's own pane ever seeing the
raw request**. This PR does not build that short-circuit. Today, EVERY
`transcript_request` — regardless of the responding agent's configured
mode — is delivered into that agent's pane like any other jekt, correctly
tiered (`TIER=sensitive` always; `ESCALATE=required` surviving a verified
sender specifically when the mode is `ask` or an un-granted
`trusted_peers` requester).

This is a deliberate, safety-conscious scope cut, not an oversight:

- The confirmed jekt rules (the actual reason this needed a repo-owner
  conversation) are fully and correctly enforced regardless of whether
  auto-resolve exists. Building the rules first, correctly, independent of
  the UX convenience layer, means the security-critical piece isn't
  entangled with — or gated behind — the larger remaining feature.
- "Always visible to a human/agent" is strictly MORE cautious than
  "sometimes silently auto-resolved" — deferring the silent path never
  creates a gap; at worst it means `private`-mode agents see a request
  they'll eventually be able to auto-deny invisibly, and have to deny it
  by hand meanwhile (a UX rough edge, not a disclosure risk).
- `db_conversation_trust_grants` and the visibility-mode lookup are ALREADY
  built and tested (§1.2, §1.4) — the auto-responder is "wire the existing
  pieces into a short-circuit before `inject_message`," not new plumbing.

**Also deferred, per the original design spec's own scope (unchanged from
`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`):**
- `muxspect conversation-requests` (list/approve/deny CLI for `ask` mode).
- `RequestTranscript`/`PollTranscriptRequest` MCP tools (the requester
  side — nothing constructs/sends a `transcript_request` yet).
- Per-source-agent rate limiting specific to `transcript_request` (the
  existing global 10/sec `RateLimiter` still applies to ALL injections,
  including these, but a dedicated narrower limit isn't built).
- Phase C (WAN) — tracked separately.

## 4. Testing

- `agentmux-common::transcript_request`: 8 unit tests — round-trips for
  request/ok/denied/error responses; ordinary free text and unrelated JSON
  never misparse as either shape; malformed JSON never panics; a request
  never parses as a response and vice versa.
- `backend::storage::conversation_trust_grants`: 10 unit tests — check/add/
  revoke/list, tier scoping, per-agent scoping, case-insensitivity,
  idempotent granting, idempotent revoking of a never-granted peer.
- `backend::reactive::tests` (handler-level): 4 new tests — rule 1 forces
  sensitive even on host tier with clean content; rule 2's forced
  escalation survives a verified sender; the mirror-image case (not
  forced) still relaxes normally for a verified sender; an unverified
  sender still requires stop regardless of the forced-escalate flag.
- `server::reactive::transcript_request_tier_resolution_tests`
  (caller-level, real `Store`): 8 new tests — ordinary messages never set
  either field; `private`/`ask`/`trusted_peers` (granted and un-granted)
  resolve correctly; a grant is tier-scoped (a LAN grant doesn't cover
  WAN); lookup matches by slug, not display name; an unknown target agent
  fails closed.
- Full `agentmux-srv` suite: 2744 passed, 0 failed, 6 pre-existing ignores.
  Full `agentmux-common` suite: 161 passed, 0 failed.
- Schema migration: `db_agent_definitions`/`db_agents` ALTER + new
  `db_conversation_trust_grants` CREATE TABLE, both idempotent
  (duplicate-column/IF-NOT-EXISTS tolerant), verified via the full existing
  `backend::storage::agents` test suite (99 tests) passing unchanged after
  the schema bump.
