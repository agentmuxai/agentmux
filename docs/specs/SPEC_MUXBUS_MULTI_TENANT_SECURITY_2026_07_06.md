# Plan: MuxBus multi-tenant security — current state and path to production isolation

**Status:** Draft — audit complete. **Update 2026-07-23: Phase 1 below has
since landed** (agentmux-cloud commit `40a2fc4`, #25, merged 2026-07-07, the
day after this doc was written) — per-agent Cognito client provisioning and
a server-side `checkAgentBinding()` check exist on all five reactive routes
in `agentmux-cloud`'s `muxbus/server/src/index.ts`. It currently runs
**log-only**: mismatches are logged, not rejected, pending the
`ENFORCE_AGENT_BINDING` flag (unset in every deployed environment). See
`agentmux-cloud`'s `muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`
for the implementation's own status tracking. Phases 2-5 below remain
unstarted.
**Author:** Agent3
**Date:** 2026-07-06
**Related:** agentmuxai/agentmux-cloud#2 (open, proposes the core fix),
SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md, #1916,
agentmux-cloud's `muxbus/SPEC_AGENT_PUBLIC_ID_2026_06_21.md`,
a5af/shared-infrastructure#372, `agentmux-docs` `security/trust-model.md`.

## TL;DR

MuxBus **authenticates** callers (a valid Cognito/legacy token) but does not
**authorize** which `agent_id`s a caller may address. Any holder of a valid
muxbus credential can inject to, poll, or claim pending messages for **any**
`agent_id` platform-wide, regardless of account. This is a stated non-goal in
agentmux-cloud's own design docs, not an overlooked bug — but it means MuxBus
in its current form is not safe for untrusted multi-tenant use. This is
already independently tracked in `agentmux-cloud#2`; this doc consolidates
that with everything else found across all four related repos and lays out a
phased path to close it.

## Current state, tier by tier

### Tier 1 (same sidecar, in-process)
Trust boundary is the process itself. No cross-tenant concern — a single
process is single-user by construction.

### Tier 2 (same host, different sidecar/channel)
Loopback HTTP + a per-launch `auth_key` UUID read from
`{data_dir}/agents/{agent_id}.json` (protected only by OS file ACLs).
Documented threat model (`agentmux-docs` `security/trust-model.md`): relies
on file permissions; explicitly **not** designed for hostile multi-tenant use
on a shared machine. Separate, already-tracked *reliability* (not security)
gap: **#1916** — Tier 2/3 delivery misses across channels because the local
agent registry is siloed per-channel.

### Tier 3 (LAN)
mDNS discovery + a per-instance `auth_key` broadcast **in plaintext** in the
mDNS TXT record (`agentmux-srv/src/backend/lan_discovery.rs`, ~line 331).
This is explicitly documented in-code as an *intentional* trade-off, not an
oversight: anyone who can already intercept LAN mDNS multicast can intercept
the HTTP traffic too, so the auth_key adds no real protection beyond casual
discovery — and the feature is opt-in only (`network:lan_discovery`, default
`false`).

### Tier 4 (WAN/cloud) — the actual gap

Confirmed by reading both sides directly: the client
(`agentmux-srv/src/muxbus/cloud_subscriber.rs`) and the server
(`agentmux-cloud`: `muxbus/server/src/index.ts`, `store.ts`,
`infrastructure/lib/constructs/muxbus-tables.ts`).

- **Authentication exists.** Every request needs a valid bearer token —
  Cognito user JWT, an M2M client-credentials token, or (for the GitHub
  webhook consumer) one shared legacy API key. `auth.ts`'s
  `ACCOUNT_REGISTRY_TABLE` is a billing-tier lookup, not an agent-ownership
  registry.
- **Authorization of `agent_id` does not exist anywhere:**
  - `POST /reactive/inject` (`muxbus/server/src/index.ts:279-344`) writes an
    injection keyed on the raw `target_agent` string from the request body,
    with no check that the caller's account owns that agent_id.
  - `GET /reactive/pending/:agent_id` and `POST /reactive/ack` /
    `/reactive/release` only check that the caller's **self-declared**
    `X-Agent-ID` header string-matches the target — there is no
    cryptographic binding between that header and the bearer token's
    identity.
  - The WebSocket `connectionsTable` has **no** agent/account column at all,
    by design: the wake broadcast is zero-metadata and fans out to every
    open connection on the fleet — the table only needs to know a socket is
    open, not who it belongs to.
  - `agent_id` is a flat, global, self-registered string (first-write-wins),
    with a hardcoded flat namespace of known peer names
    (`consumers/github/agent-mapping.ts`) shared platform-wide — no
    per-account namespacing anywhere.
- **This is a stated non-goal, not an oversight.**
  `muxbus/SPEC_AGENT_PUBLIC_ID_2026_06_21.md` (in agentmux-cloud) explicitly
  lists as non-goals: "No registry service — DynamoDB auto-registration is
  sufficient" and "No cryptographic identity / attestation — out of scope
  for this spec."

**Net exploitable behavior:** any holder of a valid muxbus credential can set
`X-Agent-ID: <someone-elses-agent>` and:
1. Read and claim pending injections addressed to that agent (interception).
2. Inject a jekt claiming `source_agent: <anyone>`, targeting any other agent
   (spoofing).
3. Do both **cross-account** — there is no account/org boundary anywhere in
   the schema today.

### Existing partial mitigation: JEKT trust tiers

The `[JEKT:FROM=... TIER=... TRUST=...]` marker convention (this repo's
CLAUDE.md, shipped in PR #1876) — auto-escalating messages containing
credential/destructive keywords to SENSITIVE, and always labeling LAN/WAN
messages `TRUST=network-claimed` — is a real, already-shipped mitigation.
But it is a **labeling and human-confirmation layer**, not an authentication
fix: it correctly assumes network-tier senders are unverified and asks the
recipient (agent or human) to be skeptical. It cannot by itself prevent the
underlying spoofing/interception at the transport layer described above.

## Prior tracking — this is not a new discovery

- **agentmuxai/agentmux-cloud#2** (open) — "muxbus Cognito auth — federated
  login, account-gating, per-agent credentials." Already states plainly:
  *"Agent identity: X-Agent-ID header — unverified, anyone can claim any
  agent"* and *"Account gating: None ... A compromised agent token
  compromises every agent. There is no kill switch per user."* This is the
  closest thing to an accepted, correctly-scoped proposal for the core fix —
  it just hasn't been implemented yet.
- **a5af/shared-infrastructure PR #372** (merged) — a live, real-world
  instance of the exact gap: the GitHub-webhook muxbus consumer trusts the
  `<!-- agentmux:agent_id=... -->` PR-body tag from *any* PR (no
  verification that the tagging PR/agent owns that id) and posts using one
  shared `muxbus-api-key`. This is the same tag convention this repo's
  CLAUDE.md instructs every agent to add to its own PRs.
- **SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md** — separately
  documents that "muxbus credentials are global across channels," a related
  but distinct symptom of the same missing-scoping root cause. Its proposed
  fix (atomic claim-before-deliver) solves *exactly-once delivery*, not *who
  is allowed to claim* — complementary to, not overlapping with, this doc.
- **#1916** — a reliability (not security) gap in the same delivery-hierarchy
  code; shares the "nothing arbitrates agent_id identity" root cause but is
  about local Tier 2/3 delivery misses, not cross-account authorization.
- **`agentmux-docs` `security/trust-model.md`** — already states outright
  that "AgentMux is not designed for hostile multi-tenant deployments." This
  audit doesn't contradict that disclosure; at the time of writing it
  confirmed "multi-user isolated and secure channels" was genuine,
  unstarted work — Phase 1 below has since shipped log-only (see the status
  update above), closing the *identity-binding* half of the gap in code
  while leaving it unenforced.

## Path to production: multi-user isolated channels

Phased, ordered by what unblocks what:

**Phase 1 — bind agent identity to the credential, not a client-supplied
header.** Replace the shared-account bearer token + self-declared
`X-Agent-ID` model with per-agent credentials (e.g. a Cognito custom claim or
a per-agent JWT scoped to exactly one `agent_id` at issuance). This directly
closes the impersonation hole and is exactly what `agentmux-cloud#2` already
proposes — the fastest path to real progress is picking that issue up, not
re-scoping from scratch.

**Phase 2 — add an agent_id ownership record, enforced server-side.**
`agentsTable` needs an `account_id`/`owner_sub` column, written at first
registration and checked on every `/reactive/inject`, `/pending`, `/ack`,
`/release` call. First-registration-wins is fine as the ownership rule; the
missing piece is *enforcing* it thereafter, not changing how ownership is
established.

**Phase 3 — namespace agent_ids per account.** Once ownership exists, stop
treating `agent_id` as a flat global string. Either enforce per-account
uniqueness (so collisions across *different* accounts become impossible, not
just discouraged) or move to a compound key (`account_id:agent_id`)
end-to-end, updating the GitHub consumer's flat `agent-mapping.ts` and the
PR-body tag convention accordingly.

**Phase 4 — extend JEKT trust tiers from "labeled" to "verified."** Once
Phase 1 exists, `TRUST=network-claimed` can become a real cryptographic claim
(message signed by the sender's per-agent credential, verified before
display) instead of a label asking the recipient to be skeptical. This turns
today's human-in-the-loop mitigation into an actual guarantee for the
sender-identity half of the trust decision.

**Phase 5 — per-account kill switch and audit log.** `agentmux-cloud#2` calls
this out directly ("no kill switch per user"). Once Phase 2's ownership
record exists, revoking/rotating one account's access without affecting
others becomes possible; pair it with an audit log of inject/claim events
keyed by the new owner field for incident response.

**Explicitly out of scope / already-acceptable trust boundaries:**
- Tier 1/2 (same host) — a single-user-desktop trust model is correct here;
  this is a desktop app, and OS-level file ACLs are the right boundary, not
  something to retrofit cloud-style tenancy onto.
- Tier 3 (LAN) plaintext `auth_key` — already an intentional, documented
  trade-off matching the Tier 2 trust assumption; only worth revisiting if
  genuinely hostile shared-network LAN use becomes a real target scenario.

## Immediate next step

Pick up **agentmux-cloud#2** as Phase 1 rather than opening a new issue — it
already scopes the core fix (per-agent Cognito credentials) accurately.
Phases 2–5 above can be filed as explicit follow-up issues referencing it
once Phase 1 lands.

## Open questions

1. Does per-agent Cognito credential issuance (Phase 1) require a new
   provisioning flow (who mints the per-agent JWT — the desktop app at agent
   creation time, or a server-side endpoint the app calls?), or can it reuse
   the existing PKCE login flow with an added custom claim?
2. For Phase 3's namespacing, does the GitHub PR-body tag convention need to
   change (e.g. `agentmux:agent_id=account/agent`), and if so, is that a
   breaking change for any external consumer of that tag format?
3. Should Phase 5's audit log live in `agentmux-cloud` (DynamoDB + a query
   endpoint) or ship events to an existing observability pipeline
   (shared-infrastructure already has patterns for this via SNS fan-out)?
