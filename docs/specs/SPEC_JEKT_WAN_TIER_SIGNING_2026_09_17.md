# SPEC: general agent-to-agent WAN-tier jekt signing

**Date:** 2026-09-17
**Status:** proposed — nothing in this document has shipped. Verified against
`agentmux` @ `f0a805d01` and `agentmux-cloud` @ `8773d4b` (both `main`,
2026-09-17).
**Tracks:** GitHub issue #2586 ("jekt: extend cryptographic signing to LAN tier
and general agent-to-agent WAN traffic") — the **WAN half**. The LAN half
shipped (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`); this closes the other.
**Revision history:** revised 2026-09-17 after Codex review on PR #3298, which
found seven P1s and one P2 — all verified against code and all accepted. The
substantive corrections: the cloud-forgery guarantee was an overclaim (§3.1,
§3.6); verification was specified on a code path ordinary WAN delivery never
takes (§3.4.2); the signed msgid/timestamp do not survive muxbus (§3.4.1); the
publish path could never fire for an already-provisioned agent (§3.2); signing
cannot be conditioned on a delivery path MCP doesn't know (§3.3); a throttled
lookup mapped to fail-open (§3.4); tenant scoping needs a *recipient* address,
not just a sender account (§2.1.1); and trusted-peer grants are not
account-scoped, which W2's duplicate names turn into a disclosure bypass
(§3.5.1). Anyone reading the PR's first commit should read this file instead.
**Builds on:**
- `SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` — the audit that made WAN
  signing conditional on tenant isolation. **§5.1's blocking finding is now
  resolved in code** (see §1.2); §6.2's proposed key design is **superseded**
  by §3.1 below.
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` — the per-agent Ed25519 pattern
  this mirrors, with a different key-distribution mechanism (§3.2).
- `SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md` — the rotation design WAN
  keys *can* adopt and LAN keys structurally could not (§3.6).
- `ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md` — the route-gate vs
  message-authentication distinction this spec sits entirely on the second
  side of.
- `agentmux-cloud/muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md` —
  the credential-binding work whose **status header is now stale in three
  ways** (§1.2).

---

## 0. TL;DR

1. **The prerequisite that blocked this in August is largely built now.** The
   account-scoped Cognito redesign (`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md`
   §5.1) landed in `agentmux-cloud` as `agent-ownership.ts` + the
   `AgentOwnershipTable`, and the desktop's 403-recovery gap (§5.2) is closed.
   What remains is **one flag flip plus one real migration**, not a redesign.
2. **One hard blocker remains, and it is the same CRITICAL finding from
   August: `agent_id` is still a flat global namespace at the storage layer.**
   `getPendingInjections` still queries `target_agent-created_at-index` on a
   bare `target_agent` with no account attribute. Until that is scoped,
   "verify agent `atlas`'s signature" has no well-defined answer — there can
   be several unrelated `atlas`es. **WAN signing cannot be correct before
   this lands** (§2.1); this is a dependency, not a nicety.
3. **The design changes from what the August spec proposed.** §6.2 there
   suggested a server-minted shared signing secret per `(account, agent_id)`.
   This spec uses **Ed25519 with the private half never leaving the sending
   srv instance** — the cloud stores only public keys. That removes what would
   otherwise be the single largest recurring cost line in this feature (§5.2:
   a Secrets Manager entry per agent), and it narrows a muxbus compromise from
   *silent* forgery to forgery that requires **rewriting a published key** —
   which §3.6's continuity chain makes detectable. It does **not** make the
   cloud structurally unable to impersonate an agent; see §3.1 for the precise,
   weaker claim that is actually true.
4. **The cloud's ownership registry is the key directory — not, by itself, a
   trust root.** Unlike LAN, which had to fall back to trust-on-first-use
   pinning because mDNS is unauthenticated, WAN has a server-authoritative
   answer to "which account owns this `agent_id`" — the same table
   `checkAgentBinding` already consults. But a directory the cloud can rewrite
   is only as trustworthy as the cloud, so §3.6 adds key-continuity signing
   plus receiver-side last-known-good pinning on top.
5. **Incremental cloud cost is effectively zero** — no new table, no new
   Cognito clients, no new secrets, sub-cent per account per month. The real
   resource limits are pre-existing and are called out honestly in §5.

---

## 1. Current state, verified

### 1.1 What exists

| Tier | Mechanism | Proves | Marker |
|---|---|---|---|
| host | HMAC-SHA256, per-agent key (`AGENTMUX_JEKT_KEY`) | claimed `source_agent` really sent it | `TRUST=host-verified` |
| channel | Ed25519, per-agent, host-global registry | same, across instances on one machine | `TRUST=channel-verified` |
| LAN | Ed25519, per-agent, pubkey pulled from the hosting peer + TOFU pin | same, across the machine boundary | `TRUST=lan-verified` |
| WAN (reagent only) | Ed25519 against a **pinned service** key | only that AgentMux's own GitHub-review service sent it — **not** per-agent identity | `SIG=verified` |
| WAN (everything else) | **nothing** | nothing | `TRUST=network-claimed` |

Confirmed unbuilt: no `wan_sig`, `wan_verified`, or `wan-verified` token
appears anywhere in `agentmux-srv`, `agentmux-common`, or `agentmux-mcp`. An
arbitrary WAN jekt's `source_agent` is exactly as forgeable as it has always
been.

### 1.2 What changed since the 2026-08-13 audit (and what didn't)

Three claims that were true in August are no longer true, and one still is.
All four are verified against current `main`, not inferred from doc status
lines:

| August finding | Status today | Evidence |
|---|---|---|
| §5.1 — per-agent Cognito clients can't scale (100 app clients/pool); needs an account-scoped client + ownership lookup | **Resolved in code.** One M2M client per account; binding is now "does this account own the claimed `agent_id`" against a DynamoDB table | `muxbus/server/src/agent-ownership.ts`, `agent-binding.ts:30-58`, `muxbus-cognito.ts:79-88` (`AgentOwnershipTable`, PK `account_user_id` / SK `agent_id`) |
| §5.2 — desktop treats only 401 as a credential problem; a 403 binding rejection silently stalls delivery | **Resolved.** 401 and 403 both trigger credential recovery, with deliberately different outcomes per path | `agentmux-srv/src/muxbus/cloud_subscriber.rs:607-671` (`is_credential_rejected`, `shared_token_rejection_outcome`) |
| PLAN's own header: "Step 2 (desktop per-agent token fetch) has not shipped" | **Stale — it shipped.** Provisioning + token fetch + cooldown + cache invalidation are live | `agentmux-srv/src/muxbus/agent_credentials.rs` (337 lines) |
| §1.1 — `agent_id` is a flat global namespace at the storage layer, zero tenant scoping | **STILL TRUE.** No account attribute in any injection key condition | `muxbus/server/src/store.ts:284-285` — `IndexName: 'target_agent-created_at-index'`, `KeyConditionExpression: 'target_agent = :agent'` |

Also still true, and relevant to sequencing:

- **`ENFORCE_AGENT_BINDING` is set nowhere.** It appears only in
  `agent-binding.ts` itself and in comments — no CDK stack, deploy script, or
  environment sets it. Binding is built, correct, and inert.
- **Legacy shared-secret auth is still accepted** and still exempt from
  binding entirely (`auth.ts:196-198` returns `{ mode: 'legacy' }`;
  `checkAgentBinding` returns early for any non-`cognito` mode).
  `DISABLE_LEGACY_AUTH` exists as an escape hatch and is likewise set
  nowhere.

**The honest summary: the August roadmap's "Phase 4" (this spec) is no longer
blocked on a redesign. It is blocked on one migration (tenant scoping) and
two flag flips that nobody has been able to safely make yet.**

---

## 2. Prerequisites — what must be true before this is meaningful

### 2.1 Hard blocker: tenant-scoped injection storage (August P0-1)

This is not "should also be fixed." It is load-bearing for the mechanism in
§3, for a specific reason:

A WAN verifier must answer *"is this signature valid for the key belonging to
the agent that sent it?"* That requires resolving `source_agent` to exactly
one public key. With a flat `agent_id` namespace, two accounts can both own an
agent named `atlas`, and the lookup is ambiguous by construction. Any
disambiguation invented at the verification layer would be guessing about
identity — the precise failure mode signing exists to eliminate.

With per-account scoping, the sending account is resolved **server-side from
the authenticated caller** at inject time, stored on the injection row, and
delivered alongside the message. The verifier then looks up
`(sender_account, source_agent)` — exactly one key, no ambiguity.

#### 2.1.1 The recipient side needs an *address*, not just a name (Codex P1)

The paragraph above solves the sender half only, and the sender half is the
easy one — the authenticated caller identifies it for free. **The recipient
half has no such source.** `getPendingInjections` is keyed on `target_agent`;
to scope that index per tenant, the cloud must know which account owns the
*target*, and a bare name cannot supply it: an ownership table keyed
`(account_user_id, agent_id)` cannot reverse-resolve `atlas` to one account
once duplicate names across accounts are permitted — which is precisely the
state tenant scoping creates.

So W2 cannot simply add a scoping attribute derived from the caller. It needs
an **authenticated destination-account component in the address itself**.
Three options, in rough order of preference:

1. **Account-qualified target** (`atlas@<account-handle>`), resolved and
   validated cloud-side at inject time. Needs a stable, non-PII account
   handle — the Cognito `sub` works but is opaque; an account-chosen handle is
   friendlier and is itself a new uniqueness namespace to administer.
2. **Same-account-by-default**, with cross-account sends requiring an explicit
   destination account. Cheapest migration (every existing send is
   same-account, so the default preserves today's behaviour), and it makes
   cross-account delivery an opt-in surface rather than an accident.
3. **Directory lookup with an explicit disambiguation error** when a name is
   ambiguous. Simplest to build, worst to use — it turns a name collision into
   a delivery failure for the *innocent* party.

**Note that `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` does
not solve this.** It is Draft with no code landed, and its own scope line
restricts it to *display naming* — explicitly "does not touch
`AGENTMUX_AGENT_ID`, jekt signing identity, or any routing/security identity."
Its WAN qualifier is a label for humans, not an authenticated address. Whoever
does W2 owns this decision; it should not be assumed borrowed from that spec.
This is open question #6 (§9).

Follow the migration pattern this codebase already has precedent for
(`SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`, and §6.3 of the
August spec): add attribute + GSI, dual-write, cut reads over, backfill, drop
the old GSI. Note §6.3's own sequencing caveat still applies — a legacy-key
caller has no resolvable account, so the legacy mode must be scoped or retired
*before* the scoped-read cutover, not after.

### 2.2 Soft prerequisite: `ENFORCE_AGENT_BINDING=true`

Signing and binding answer different questions, and this spec does **not**
strictly require the flag to be on: a valid signature proves the sender
regardless of whether the cloud also rejected mismatched claims. But leaving
binding inert means an unsigned WAN jekt can still claim any agent name under
any account, so the *unsigned* path stays as weak as it is today and
`TRUST=wan-verified` becomes a label that only well-behaved senders opt into.

Recommend flipping it during Phase W0 (§4), which is now much cheaper than it
was in August: the ownership check treats an absent row as "not yet migrated"
rather than a mismatch, treats a transient DDB error as "unknown, don't
enforce," and the desktop recovers from a 403. The remaining work is a burn-in
read of the `[agent-binding]` warn logs, not new engineering.

### 2.3 Not a prerequisite

Retiring legacy auth is not required for *this* mechanism to be correct (a
legacy caller simply produces unsigned WAN traffic, which stays
`TRUST=network-claimed`). It **is** required for §2.1's read cutover. Keep the
dependency stated accurately rather than bundling it here.

---

## 3. Design

### 3.1 Key material: per-agent Ed25519, private half never leaves the host

New table `db_agent_wan_keys` in `agentmux-srv`, mirroring `db_agent_lan_keys`
(`agent_lan_keys.rs`) exactly — `agent_id` TEXT PK (lowercased), `public_key`
TEXT (base64, 32 bytes), `private_key` TEXT (base64, 32-byte seed),
`created_at` INTEGER. `agent_wan_key_ensure(agent_id)` mints on first use with
the same race-safe `INSERT OR IGNORE` + re-read pattern.

The private half is injected into that agent's own `agentmux-mcp` process env
at spawn as `AGENTMUX_WAN_KEY`, alongside `AGENTMUX_JEKT_KEY` and
`AGENTMUX_LAN_KEY` — same spawn-time-only, never-over-RPC, never-into-another-
agent's-env guarantee.

**This supersedes `SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §6.2**,
which proposed a random per-`(account, agent_id)` *shared secret* minted
cloud-side at provisioning time and returned to the desktop. Three reasons to
reject that now:

1. **A symmetric scheme hands the cloud silent forgery.** The verifier would
   hold the same secret the signer does, so a compromised muxbus could mint
   valid messages for any agent with no observable trace.

   **What Ed25519 actually buys, stated precisely (Codex P1 — an overclaim in
   the first draft of this spec):** it does *not* make the cloud unable to
   impersonate an agent. The cloud serves the directory, so a compromised
   muxbus can overwrite `wan_public_key` with a key it holds the private half
   of, sign as that agent, and have receivers fetch and trust the replacement.
   The real gain is narrower and still worth having: forgery now requires
   **mutating durable, observable state** (a key replacement) rather than
   quietly using a secret it already legitimately holds. §3.6's continuity
   chain plus receiver-side last-known-good pinning is what converts that
   mutation into something a receiver can *detect*. Without those two, this
   bullet claims nothing over the symmetric design.
2. **Cost.** A secret per `(account, agent_id)` in Secrets Manager is the
   single largest recurring line item this feature could have had (§5.2).
   Public keys on an existing DynamoDB row cost approximately nothing.
3. **Consistency.** LAN and cross-channel signing are both already Ed25519
   per-agent. A third scheme with different forgery properties is a
   review-burden trap.

**Separate keypair from LAN, deliberately.** Reusing `db_agent_lan_keys` for
WAN would be tempting and is wrong on two counts:

- `signed_material(msgid, source_agent, target_agent, ts_secs, message)`
  (`agentmux-common/src/jekt_sign.rs:52-54`) carries **no tier or domain
  separator**. With one key across both tiers, a captured LAN signature is a
  byte-identical valid WAN signature for the same message — a free cross-tier
  replay.
- LAN public keys are **permanently TOFU-pinned** by every peer that has ever
  seen them (`db_lan_peer_pubkey_pins`, no expiry, no re-pin path), which is
  exactly why `SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md` §4 declined to
  rotate them. WAN keys have an authoritative registry and *should* rotate
  (§3.6). One key cannot have both lifecycles.

WAN signatures therefore sign a domain-separated material string —
`"wan|v1|" + signed_material(...)`. Host and LAN material stay byte-identical
to what ships today; this spec introduces no change to any already-deployed
verifier.

### 3.2 Public key distribution: the ownership registry is the PKI

LAN had no authority to ask, so it pinned. WAN has one: the same
`AgentOwnershipTable` row (`account_user_id` / `agent_id`) that
`checkAgentBinding` already consults to decide whether an account may claim an
agent name.

- **Publish — on its own path, NOT on provisioning (Codex P1).** Add a
  `wan_public_key` attribute plus `wan_key_version` and `wan_key_created_at`
  to that row, written by a dedicated authenticated route
  **`PUT /agents/:agent_id/pubkey`**.

  The first draft of this spec proposed riding the existing
  `POST /agents/provision` call. **That cannot work**, and the reason is
  structural rather than incidental: `ensure_agent_credential`
  (`agent_credentials.rs:88-104`) calls `provision_agent_client` *only* when
  `agent_credential_load` returns nothing, and credentials are deliberately
  provisioned once per agent and cached. Every already-provisioned agent —
  i.e. every agent that matters, since they all provisioned before this ships
  — would therefore mint a local WAN key and **never** publish it, and no
  rotation (§3.6) would ever reach the directory either. Receivers would see a
  signature they cannot check, forever, and the feature would silently do
  nothing.

  **Version-synchronised publish:** srv holds `wan_key_version` locally
  alongside the keypair; on each `sync_agent_reactive` cycle it compares its
  local version against the last successfully published one (cached locally,
  not re-fetched) and issues the `PUT` only on a mismatch. Steady state is
  zero extra calls; a fresh mint or a rotation is exactly one. Publish failure
  must be non-fatal and retried on the next cycle — an unpublished key
  degrades to unsigned-equivalent (`wan_verified = None`), never to a
  verification failure.
- **Fetch.** New authenticated route
  `GET /agents/:agent_id/pubkey?account=<sender_account>` returning
  `{ agent_id, account_user_id, wan_public_key, wan_key_created_at }` or 404.
  Cached in srv with a short TTL (§3.6 explains why it must be short, unlike
  LAN's).
- **No mDNS analogue, no TOFU, no pin table.** A pin exists to substitute for
  an authority; here there is one, and inventing a second source of truth
  would create precisely the "pinned key vs rotated key" false-positive
  described in the 09-14 spec §4.

A row's mere existence already means ownership (that module's own doc
comment), so a key found on the row is a key the cloud asserts belongs to that
account's agent. The trust anchor is therefore the Cognito account boundary —
the same anchor `checkAgentBinding` and billing already rest on. Say that
plainly rather than implying cryptography creates trust it doesn't.

### 3.3 Signing (client side, `agentmux-mcp`)

Mirror the existing `AGENTMUX_JEKT_KEY` / `AGENTMUX_LAN_KEY` closures in
`agentmux-mcp/src/main.rs` — and mirror them **unconditionally** (Codex P2).

The first draft said "when the resolved delivery path is WAN," which has no
implementable call site: `agentmux-mcp` posts every send to the local reactive
endpoint, and srv only afterwards decides among local, cross-channel, LAN, and
cloud delivery. `sign_outgoing_jekt` already computes `lan_sig` for every
outgoing message for exactly this reason, and says so in its own doc comment —
"signing `lan_sig` alongside a message that's actually delivered host or WAN
costs nothing — srv only ever consults `lan_sig` when it has" the tier to
match. `wan_sig` joins that tuple on identical terms: computed always,
consulted only for a **server-derived** WAN delivery.

`OutgoingJektSignatures` therefore becomes
`(request_id, ts_secs, jekt_sig, lan_sig, source_channel, channel_sig,
wan_sig)`. Note this makes MCP the origin of both the `msgid` and `ts_secs`
that the signature covers — which §3.4.1 shows is the crux of getting the
signature to survive the cloud.

**Best-effort, exactly as every other tier is**: a missing key must never
block sending. An agent that hasn't been respawned since this ships has no WAN
key yet and its traffic stays `TRUST=network-claimed` — the same graceful
degradation host and LAN tiers already have, and the reason this can roll out
without a flag day.

### 3.4 Verification (server side, `agentmux-srv`)

#### 3.4.1 The signed tuple must survive muxbus (Codex P1)

This is the finding that most changes the shape of the work, and the first
draft missed it entirely by listing only `wan_sig`/`wan_verified` as new
fields.

The signature covers `signed_material(msgid, source_agent, target_agent,
ts_secs, message)`. Neither `msgid` nor `ts_secs` survives the round trip as
signed:

- `cloud_subscriber.rs:948` sets `request_id: Some(inj.id.clone())` — the
  **cloud-generated** injection id (`inj-<epoch>-<rand>`), not the msgid MCP
  minted and signed.
- `ts_secs` is left unset on that path entirely.
- `relay_inject` carries only target / message / priority.

Verification would therefore be computed over different material than was
signed, and **every** delivered signature would fail — rendering the
active-forgery escalation on legitimate traffic, which is the worst possible
failure mode here.

**Reagent already solved this**, and its solution is the precedent to copy:
it carries `reagent_msg_id` and `reagent_ts_secs` as explicit fields
alongside `reagent_sig`/`reagent_key_id`, precisely so the verifier can
reconstruct the exact signed material regardless of what the transport did to
the envelope (`cloud_subscriber.rs:916-935`).

So the new fields are five, not two:

| Field | Source | Notes |
|---|---|---|
| `wan_sig` | client-supplied | base64 Ed25519 |
| `wan_msg_id` | client-supplied | the msgid **as signed**, independent of the cloud's injection id |
| `wan_ts_secs` | client-supplied | the timestamp **as signed** |
| `wan_sender_account` | **server-resolved** at inject time (§2.1) | which account's key to check against |
| `wan_verified` | **server-computed**, `#[serde(skip_deserializing)]` | same trust boundary as `sig_verified` / `reagent_verified` / `lan_verified` — a client must never assert its own verification result |

All four client-supplied fields must be persisted by muxbus as **opaque
storage** (the server never verifies — same posture as the four `reagent_*`
fields) and relayed through `store.ts`, `relay_inject`, and the pending-poll
response.

**Freshness:** because `wan_ts_secs` is now attacker-influenceable client
input, it needs the same bounded-age check reagent has
(`reagent_sig_is_fresh`, `REAGENT_SIG_MAX_AGE_SECS`) or a captured valid
signature is replayable indefinitely. Reuse that helper's shape.

#### 3.4.2 Where verification runs (Codex P1)

`verify_wan_signature(state, req)` uses the three-way semantics used
everywhere else in this codebase — but **it must run on the
`cloud_subscriber` path, not only on `handle_reactive_inject`.**

The first draft said "called from `handle_reactive_inject` alongside the
existing verifiers." Ordinary desktop WAN delivery never reaches that
handler: `cloud_subscriber::sync_agent_reactive` builds an `InjectionRequest`
itself and calls `handler.inject_message` directly — which is exactly why
reagent verification is *duplicated* in that subscriber rather than living
only in the HTTP handler. Specified as originally written, every real muxbus
delivery would keep `wan_verified = None` and `TRUST=wan-verified` would never
render at all.

Preferred fix: hoist all four verifiers into one shared pre-handler step both
entry points call, rather than adding a third copy of the same call sequence.
If that refactor is too large for W3, duplicate deliberately and leave a
comment naming the other call site — but the duplication is already at three
verifiers and is the reason this bug was easy to introduce.

Scoped to `delivery_tier == "wan"`, the semantics are:

1. No `wan_sig` → `wan_verified = None` ("not signed").
2. `wan_sig` present, no public key resolvable for `(sender_account,
   source_agent)` → `wan_verified = None`. Nothing to check against is not a
   failure — same as host-tier's "no key on file."
3. `wan_sig` present, key found, signature does **not** verify →
   `wan_verified = Some(false)`. **Active red flag**, same category as
   `SIG=invalid` and a failed LAN signature.
4. `wan_sig` present, verifies → `wan_verified = Some(true)`.

Two implementation constraints carried over from LAN's review, which apply
here for the same reasons:

- **Rate-limit the pubkey fan-out — and treat a throttled lookup on a *signed*
  request as a failure, not as key-absence (Codex P1).** The lookup is a
  network call keyed by a caller-controlled string; a negative cache keyed only
  on `agent_id` lets a caller force a fresh cloud round trip per request by
  varying `source_agent`, ahead of the handler's own rate limiter. Use a global
  token bucket.

  The first draft said to fail "closed to unverifiable (case 2)." **That is
  fail-open, not fail-closed**: an attacker who exhausts the bucket gets a
  presented-but-invalid signature downgraded to `wan_verified = None`, i.e.
  ordinary `TRUST=network-claimed` treatment, which is exactly the
  escalation-bypass the bucket exists to prevent. Shipped LAN code already
  gets this right — `LanPubkeyLookup::RateLimited` maps to
  `lan_verified = Some(false)` (`server/reactive.rs:478-489`) with a comment
  naming this attack. **Mirror that**: a throttled lookup for a request that
  *presented* a signature is `Some(false)`; a request with no signature is
  unaffected (still `None`, nothing was claimed). Retaining a known cached key
  and verifying against it is also acceptable and strictly better when one
  exists.

  Note for implementers: `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §2.2 still
  describes the fail-to-`None` behaviour its own implementation deliberately
  rejected. The code is correct and the spec text is stale; this draft
  inherited the stale text. Trust `reactive.rs`.
- **`delivery_tier` must stay server-derived where it can be.** §3 of the LAN
  spec established that a client-asserted tier is a way to choose which
  verifier runs. WAN arrives via `cloud_subscriber`, not a client-controlled
  local route, so the tier is already trustworthy on that path — but
  `verify_wan_signature` should follow `verify_jekt_signature`'s post-review
  shape and not depend on the tier claim being honest for its *security*
  properties.

### 3.5 Marker, tier rules, and CLAUDE.md

New `TRUST=wan-verified`, rendered by `sanitize.rs::wrap_jekt_message` when
`wan_verified == Some(true)`. Everything else on WAN keeps
`TRUST=network-claimed`.

**`TRUST=wan-verified` and reagent's `SIG=verified` are different claims and
must not be merged.** `SIG=verified` says "AgentMux's own review service sent
this"; `TRUST=wan-verified` says "this specific agent, under the account that
owns that name, sent this." Reagent traffic continues to render
`SIG=verified` with `TRUST=network-claimed` unless reagent itself also gets a
per-agent identity, which is out of scope.

Tier escalation (`handler.rs`), additive to the post-08-15 rules:

```rust
// An ACTIVE WAN verification failure is exactly as much a red flag as
// SIG=invalid, a failed LAN signature, or host-tier TRUST=unverified.
let is_wan_sig_invalid = delivery_tier == "wan" && req.wan_verified == Some(false);
let is_sensitive = is_network_tier_sig_invalid
    || is_lan_sig_invalid
    || is_wan_sig_invalid            // NEW
    || is_unverified_sender
    || matches!(declared_tier, Some(JektTier::Sensitive))
    || is_sensitive_message(&sanitized);
```

`TRUST=wan-verified` joins the **`ESCALATE=none` verified-sender set**
(`host-verified`, `lan-verified`, `channel-verified`, WAN `SIG=verified`) per
`SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md` — a
keyword-matched or self-declared-sensitive jekt from a cryptographically
proven WAN sender tags rather than stops. **The `transcript_request` exception
(`SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`) is unaffected and
continues to force `ESCALATE=required`** under `ask` / non-allow-listed
`trusted_peers`, for a verified WAN sender exactly as for any other verified
sender: identity proof answers *who is asking*, not *whether this content
should be disclosed*.

#### 3.5.1 Trusted-peer grants must be scoped to the sender account (Codex P1)

The paragraph above is true for the `ask` mode but **not** for
`trusted_peers`, and the gap is created by this spec's own prerequisite rather
than existing today.

`db_conversation_trust_grants` has `PRIMARY KEY (agent_id,
granted_peer_agent_id, tier)` (`migrations.rs:1024-1030`) — the granted peer is
identified **by bare name plus tier, with no account**. That is sound while
`agent_id` is globally unique. Once W2 permits duplicate names across
accounts, it is exploitable: an attacker registers `atlas` under *their own*
account, publishes a key they control, and sends a `transcript_request` that
verifies perfectly under their own registered key. It arrives
`TRUST=wan-verified` from "atlas" at tier `wan`, matches the victim's existing
grant for a completely different `atlas`, and is **auto-approved — the
disclosure stop is suppressed by a valid signature proving the wrong
identity.**

Note the perverse ordering: signing makes this *worse*, not better. Today such
a request arrives unverified and the `ask`/non-allow-listed path stops it.

**Requirement:** thread `sender_account` into both the grant's persisted
identity and the authorization lookup — `PRIMARY KEY (agent_id,
granted_peer_account, granted_peer_agent_id, tier)`, with existing rows
migrated to the owning account for host/LAN/channel tiers and **existing `wan`
grants invalidated rather than guessed at**. This holds even though §9.5
recommends keeping the account out of the rendered marker: the marker is a
display decision, the grant is an authorization decision, and they do not have
to agree.

This must land **with** W3, not after it — shipping `TRUST=wan-verified`
before the grant lookup is account-scoped actively regresses a security
property the 08-22 spec established.

Clean-content WAN traffic is already not forced sensitive (the 08-15
narrowing), so this adds no new relaxation — it changes a label from *claimed*
to *proven*, and adds one new active-forgery case.

CLAUDE.md's jekt section (repo copy and the workspace copies) must gain
`TRUST=wan-verified` in three places: the delivery-tier explainer, the
`TIER=info`/`coord` default list, and the `ESCALATE=none` list — plus the new
forced-sensitive bullet. Per that section's own standing rule, this documents
shipped code; it must not land ahead of the implementation.

### 3.6 Rotation — the thing LAN could not have

Because there is a registry rather than permanent pins, a rotated WAN key is a
**republish**, not a spoofing alarm. WAN keys can therefore adopt the lazy 24h
TTL rotation of `SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md`:
`agent_wan_key_ensure` rotates at the next spawn once the row is older than
the TTL, and a live process keeps signing with the key already in its env. The
republish goes through §3.2's version-synchronised `PUT` — **not** the
provisioning call, which fires once per agent and would never run again.

**Key continuity — what makes §3.1's weaker claim true.** A republish is
indistinguishable from a cloud-side key substitution unless the new key proves
it descends from the old one. So each rotation publishes
`wan_key_continuity_sig`: **the new public key, signed by the previous private
key**, plus the version it supersedes. A receiver that holds a last-known-good
key for `(sender_account, source_agent)` — cached locally, `db_wan_peer_keys`,
mirroring `db_lan_peer_pubkey_pins`'s role but *without* its permanence —
verifies the chain before accepting a replacement:

- Chain verifies → accept the new key, advance the pin. Ordinary rotation.
- Chain missing or fails → **do not accept silently.** Treat as
  `wan_verified = Some(false)` for messages under the new key and log loudly:
  either the directory was rewritten, or an agent's key was re-minted without
  continuity (e.g. its local DB was wiped).
- No prior key held → accept on first observation (unavoidable TOFU, same
  residual risk LAN accepts, and narrowed here because the directory itself is
  authenticated).

This is the piece that converts "the cloud can rewrite the directory" from
*undetectable* to *detectable*. Without it, §3.1's bullet 1 is marketing.
A legitimate re-mint with no continuity proof needs a human-visible recovery
path — same unsolved shape as LAN's missing re-pin action, and the reason §9.7
asks for a decision rather than asserting one.

The one constraint this places on §3.2: **the receiver's pubkey cache TTL must
be shorter than the rotation interval**, and a verification failure against a
cached key should trigger exactly one forced refetch before being reported as
`Some(false)`. Without that, every rotation produces a window of false
"active forgery" alarms — the same failure the 09-14 spec refused to inflict
on LAN, arriving through a cache instead of a pin. Recommend a 1h cache TTL
against a 24h rotation, plus the single refetch-on-mismatch.

---

## 4. Phasing

| Phase | Work | Depends on | Ships value alone? |
|---|---|---|---|
| **W0** | Flip `ENFORCE_AGENT_BINDING` after a burn-in read of `[agent-binding]` warnings | nothing new | Yes — closes cross-account impersonation at the cloud API |
| **W1** | Scope or retire legacy shared-secret auth (`DISABLE_LEGACY_AUTH`, or per-consumer keys + an account binding) | W0's log evidence | Yes — closes the unmetered, unbound bypass |
| **W2** | Addressing decision (§2.1.1) **then** tenant-scoped injection storage: attribute + GSI, dual-write, backfill, read cutover, drop old GSI | W1 (legacy callers need a resolvable account first) | Yes — closes the CRITICAL cross-tenant queue collision |
| **W3** | WAN key material + version-synced publish/fetch + unconditional sign + verify on the subscriber path + signed-tuple relay + marker + **account-scoped trust grants (§3.5.1)** | **W2** | Yes — `TRUST=wan-verified` |
| **W4** | Rotation + continuity chain + cache/refetch semantics (§3.6) | W3 | Hardening |

**§3.5.1 is not deferrable to W4.** Account-scoped trust grants must ship in
the same change as `TRUST=wan-verified`, because a verified signature from a
same-named agent under a *different* account would otherwise satisfy an
existing `trusted_peers` grant and suppress a `transcript_request` disclosure
stop — a regression created by W2's duplicate names and made exploitable by
W3's signatures.

W0–W2 are all *pre-existing* security debt with their own independent value;
none of them is make-work invented by this spec. W3 is the only phase that is
this spec's own subject, and it is the smallest of the five.

---

## 5. Resource and cost limitations

Grounded in this system's actual numbers (`muxbus/server/src/quota.ts`,
`docs/SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md` §4), not generic AWS
list pricing.

### 5.1 Hard ceilings that already exist

| Limit | Value | Effect on this work |
|---|---|---|
| Cognito app clients per user pool | **100** (AWS default; raisable by support ticket) | The §5.1 redesign moved the denominator from *agents* to *accounts* — so the ceiling is now ~100 **accounts**, system-wide, not 100 agents. WAN signing adds **zero** clients. But this is still a hard ceiling on customer count and should be raised deliberately, before it is hit, not during an incident. |
| `agent_provisions` quota | **50 / account / month**, unbilled, purely a rate limit | Key publication rides the provision path. An account that churns agent names (rename loops, ephemeral agents) could exhaust this and silently stop publishing new keys — degrading to unsigned, not to broken. Worth a dedicated metric before W3. |
| `jekt_messages` free tier | **2,000 / account / month**, $0.001 each beyond | Unchanged by signing — a signature is a field on an existing message, not a new message. **Pubkey lookups must not be charged against this** (§5.3). |
| WebSocket broadcast fan-out | full table `Scan` + `PostToConnection` to every open connection, per message | Pre-existing O(N) cost amplifier flagged in the cost-cap spec §6. Unrelated to signing, but it is the thing that actually scales badly as WAN traffic grows — call it out so WAN signing isn't blamed for a cost curve it didn't cause. |

### 5.2 Incremental cost of this design: approximately zero

- **No new DynamoDB table, no new GSI.** `wan_public_key` is an attribute on a
  row that already exists and is already read on the binding path. Marginal
  storage is ~100 bytes per `(account, agent)`; at any plausible scale this
  rounds to zero against DynamoDB's minimum billable item size.
- **No new Cognito clients** (account-scoped model already in place).
- **No Secrets Manager entries.** This is the cost-decisive design choice. The
  superseded §6.2 shared-secret design needed one secret per
  `(account, agent_id)`; at Secrets Manager's $0.40/secret/month, 10 accounts
  × 50 agents = 500 secrets ≈ **$200/month** — roughly 2,500× the *entire*
  current per-user AWS cost, and by itself a multiple of the $1/user/month cap
  the owner-gate spec exists to enforce. Ed25519 with host-held private keys
  avoids this line item completely.
- **Lambda / API Gateway:** one extra `GET …/pubkey` per
  `(receiver, sender-agent)` per cache TTL. With a 1h TTL (§3.6) and a handful
  of distinct correspondents, that is tens of invocations per account per
  month — well under a cent.
- **Compute on srv:** Ed25519 verification is ~50–100 µs, on a path that
  already does a network round trip. Not measurable.
- **Net:** the existing free-tier allocation costs **$0.03–0.08/account/month**
  at 100% utilization. This feature does not move that figure to a second
  significant digit, and stays comfortably inside the $1/account/month cap.

### 5.3 The one billing decision that needs a call

**Do pubkey lookups consume quota?** Recommend **no** — neither
`jekt_messages` nor a new billable resource:

- Charging them against `jekt_messages` would mean a signed conversation
  consumes budget at up to **2× the rate** of an unsigned one, i.e. the
  security feature bills users for being verifiable. That is the same class
  of mistake as the `/reactive/inject` bug the 08-14 work fixed (charging
  jekts against `drone_runs`, a 20× smaller ceiling).
- If abuse control is wanted, add an **unbilled** `pubkey_lookups` resource
  with a generous limit, mirroring `agent_provisions`' "rate limit, price 0"
  precedent — not a revenue line.

### 5.4 Engineering cost, stated honestly

W3 is **larger than the LAN signing PR**, and the first draft of this spec
understated it by assuming LAN parity. Post-review scope: two new local tables
plus a PK migration on `db_conversation_trust_grants`; a common-crate
signing/verification pair with domain separation and a freshness window; five
`InjectionRequest` fields that must be persisted and relayed end-to-end
through `store.ts`, `relay_inject`, and the pending-poll response; a key
continuity chain with its own verification path; two cloud routes plus four
new attributes; the version-synchronised publish loop; a marker value; an
escalation branch; the account-scoped grant lookup; and ideally the shared
pre-handler verifier hoist. Call it **one to two weeks**, not a few days — and
that is still the *small* phase.

**W0–W2 remain the real cost, and W2 most of all.** A dual-write /backfill/
cutover on a live multi-tenant DynamoDB table carrying paying customers'
undelivered messages is not a code change; it is a migration with a rollback
plan, a backfill that must refuse to guess (§6.3 of the August spec: rows that
cannot be confidently attributed get flagged for manual review, never assigned
to a best-guess tenant), and a cutover window. W2 also now carries the
addressing decision in §2.1.1, which is a product change with a client-visible
surface — size it accordingly. Anyone sizing "WAN signing" should be sizing
that, not §3.

---

## 6. Failure semantics

Identical three-way split to every other verifier here — no silent drop, no
silent trust:

- No signature → `wan_verified = None`, `TRUST=network-claimed`, tier per the
  08-15 narrowing (not forced sensitive by absence alone).
- Present, key unresolvable (never published, or account has no such agent) →
  `wan_verified = None`, same as above. "Nothing to check against" is not
  "check failed."
- Present, but the pubkey lookup was **rate-limited** → `wan_verified =
  Some(false)`. A skipped check on a request that claimed a signature is not
  key-absence; mapping it to `None` is the escalation bypass LAN's
  implementation already closes (§3.4).
- Present, key replaced without a verifying continuity chain →
  `wan_verified = Some(false)` plus a loud log (§3.6).
- Present, key found, fails → `wan_verified = Some(false)`, forced
  `TIER=sensitive` with `ESCALATE=required`, unconditionally.
- Present, verifies → `wan_verified = Some(true)`, `TRUST=wan-verified`, tier
  per declared/keyword rules. Never auto-relaxes below what clean unsigned WAN
  traffic already gets.

---

## 7. Out of scope

- Per-agent identity for **reagent** itself; its pinned service key and
  `SIG=verified` semantics are untouched.
- Any change to host, channel, or LAN signing, including LAN key rotation
  (still blocked on a re-pin mechanism per the 09-14 spec §4).
- End-to-end **encryption** of jekt bodies. This spec is authentication only;
  muxbus still sees plaintext message content, exactly as today.
- Revocation UX (a compromised agent key's removal path) — rotation bounds the
  window, an explicit revoke action does not exist for any tier and should be
  sized once for all of them.
- Replay protection on `createInjection` (August §6.1: no idempotency key;
  bounded severity, still open).

---

## 8. Key files

**agentmux**
- New: `agentmux-srv/src/backend/storage/agent_wan_keys.rs` (mirrors `agent_lan_keys.rs`) — keypair + `wan_key_version` + last-published version
- New: `agentmux-srv/src/backend/storage/wan_peer_keys.rs` — last-known-good peer key per `(sender_account, agent_id)`, continuity-checked (§3.6); **not** a permanent pin
- `agentmux-srv/src/backend/storage/migrations.rs` — two new tables, plus the `db_conversation_trust_grants` PK change (§3.5.1)
- `agentmux-common/src/jekt_sign.rs` — `sign_wan_jekt`/`verify_wan_jekt`, domain-separated material; reuse `reagent_sig_is_fresh`'s shape for WAN freshness
- `agentmux-srv/src/backend/reactive/types.rs` — `wan_sig`, `wan_msg_id`, `wan_ts_secs`, `wan_sender_account`, `wan_verified` (§3.4.1)
- `agentmux-srv/src/backend/reactive/handler.rs` — escalation branch (§3.5)
- `agentmux-srv/src/backend/reactive/sanitize.rs` — `TRUST=wan-verified`
- `agentmux-srv/src/server/reactive.rs` — `verify_wan_signature`; ideally the shared pre-handler hoist (§3.4.2)
- **`agentmux-srv/src/muxbus/cloud_subscriber.rs` — the primary verification call site** (§3.4.2), relay of the signed tuple, and the version-synchronised pubkey `PUT` (§3.2)
- `agentmux-srv/src/muxbus/agent_credentials.rs` — *unchanged by design*; provisioning is explicitly **not** the publish path
- `agentmux-srv/src/server/app_api/agent_open.rs` — `AGENTMUX_WAN_KEY` env injection
- `agentmux-mcp/src/main.rs` — unconditional signing in `sign_outgoing_jekt` (§3.3)
- `CLAUDE.md` + `docs/specs/ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md` — new row/values (that map's own rule: same change, not a follow-up)

**agentmux-cloud**
- `muxbus/server/src/agent-ownership.ts` — `wan_public_key`, `wan_key_version`, `wan_key_continuity_sig`, `wan_key_created_at`
- `muxbus/server/src/index.ts` — `PUT /agents/:agent_id/pubkey`, `GET /agents/:agent_id/pubkey`
- `muxbus/server/src/store.ts` — opaque pass-through of the four client-supplied `wan_*` fields; sender account + destination-account scoping on the injection row (W2, §2.1.1)

---

## 9. Open questions needing sign-off

1. **W2 scheduling.** The tenant-scoping migration touches live paying
   customers' undelivered messages. Who runs it, in what window, with what
   rollback? This is the decision that gates everything else.
2. **Legacy auth (W1).** Retire outright via `DISABLE_LEGACY_AUTH`, or issue
   per-consumer keys bound to an internal service account? The github
   consumer is the known caller; is there an unknown one? (Recommend: measure
   first — log distinct legacy callers for a week before choosing.)
3. **Cognito's 100-client ceiling.** Raise it proactively now, or wait for
   account growth to approach it? It is a support ticket, not engineering,
   and it is much cheaper before than during.
4. **Pubkey lookup billing** (§5.3) — confirm "unbilled" is the intended
   policy.
5. **Does `TRUST=wan-verified` need to name the owning account in the marker?**
   It would make cross-account provenance explicit to the reading agent, but
   the account identifier is a Cognito sub tied to a human's email — a
   privacy question, not just a UX one. Recommend: no account in the marker
   for now; the receiving *instance* can log it. Note this is independent of
   §3.5.1 — grants must be account-scoped regardless of what the marker shows.
6. **Which addressing form for W2** (§2.1.1) — account-qualified target,
   same-account-by-default with explicit cross-account sends, or ambiguity
   errors? This shapes the migration, the client API, and whether
   cross-account delivery is opt-in. Recommend option 2. **It is a product
   decision, not just a schema one**, and it gates W2.
7. **Recovery when key continuity legitimately breaks** (§3.6) — an agent
   whose local DB was wiped re-mints with no continuity proof and becomes
   indistinguishable from a directory rewrite. LAN has the same unsolved shape
   (no re-pin action). Options: a human-confirmed "trust this new key" action,
   an account-owner-authenticated reset, or accepting a loud-but-recoverable
   degradation to unverified. Needs a call before W4.

---

## 10. Sources

- `agentmux-cloud/muxbus/server/src/agent-ownership.ts`, `agent-binding.ts`, `agent-provisioning.ts`, `auth.ts`, `store.ts`, `quota.ts`
- `agentmux-cloud/muxbus/infrastructure/lib/constructs/muxbus-cognito.ts` (`AgentOwnershipTable`)
- `agentmux-srv/src/muxbus/agent_credentials.rs`, `cloud_subscriber.rs`
- `agentmux-common/src/jekt_sign.rs` (`signed_material` — confirms no domain separator)
- `docs/specs/SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` (§1.1, §5.1, §5.2, §6.2, §6.3)
- `docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`
- `docs/specs/SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md` (§4 — why LAN keys can't rotate)
- `docs/specs/SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`, `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`
- `docs/specs/ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md`
- `agentmux-cloud/docs/SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md` (§4 unit-cost derivation, §6 broadcast fan-out)
- [Quotas in Amazon Cognito](https://docs.aws.amazon.com/cognito/latest/developerguide/quotas.html) (100 app clients per user pool, default)
