# SPEC: general agent-to-agent WAN-tier jekt signing

**Date:** 2026-09-17
**Status:** proposed — nothing in this document has shipped. Verified against
`agentmux` @ `f0a805d01` and `agentmux-cloud` @ `8773d4b` (both `main`,
2026-09-17).
**Tracks:** GitHub issue #2586 ("jekt: extend cryptographic signing to LAN tier
and general agent-to-agent WAN traffic") — the **WAN half**. The LAN half
shipped (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`); this closes the other.
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
   srv instance** — the cloud stores only public keys. That makes the cloud
   structurally unable to forge an agent's identity, and it removes what
   would otherwise be the single largest recurring cost line in this feature
   (§5.2: a Secrets Manager entry per agent).
4. **The cloud's ownership registry is the PKI.** Unlike LAN, which had to
   fall back to trust-on-first-use pinning because mDNS is unauthenticated,
   WAN has a server-authoritative answer to "which account owns this
   `agent_id`" — the same table `checkAgentBinding` already consults. Public
   keys ride on that row. No TOFU, no pin-mismatch false positives, and
   **rotation is possible** (§3.6).
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

1. **The cloud would be able to forge any agent's identity.** A symmetric
   scheme requires the verifier to hold the same secret the signer does. With
   Ed25519 the cloud holds only public keys, so a compromise of muxbus
   degrades to "can drop or misroute messages" — never "can impersonate an
   agent to another account."
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

- **Publish.** Add a `wan_public_key` attribute (plus `wan_key_created_at`) to
  that row. The desktop publishes it on the existing
  `POST /agents/provision` call path — the same call
  `agent_credentials.rs::provision_agent_client` already makes — extended to
  carry the agent's current WAN public key. No new client-side call, no new
  round trip in the common case.
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
`agentmux-mcp/src/main.rs`: when the resolved delivery path is WAN, read
`AGENTMUX_WAN_KEY` and sign the domain-separated material with Ed25519.

**Best-effort, exactly as every other tier is**: a missing key must never
block sending. An agent that hasn't been respawned since this ships has no WAN
key yet and its traffic stays `TRUST=network-claimed` — the same graceful
degradation host and LAN tiers already have, and the reason this can roll out
without a flag day.

### 3.4 Verification (server side, `agentmux-srv`)

New `InjectionRequest` fields: `wan_sig: Option<String>` (client-supplied,
base64) and `wan_verified: Option<bool>` (**server-computed,
`#[serde(skip_deserializing)]`** — the same trust boundary `sig_verified`,
`reagent_verified`, and `lan_verified` already use; a client must never be
able to assert its own verification result).

`verify_wan_signature(state, req)`, called from `handle_reactive_inject`
alongside the existing verifiers, scoped to `delivery_tier == "wan"`, with the
three-way semantics used everywhere else in this codebase:

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

- **Rate-limit the pubkey fan-out.** The lookup is a network call keyed by a
  caller-controlled string; a negative-cache keyed only on `agent_id` lets a
  caller force a fresh cloud round trip per request by varying
  `source_agent`, ahead of the handler's own rate limiter. Use a global token
  bucket, failing **closed to "unverifiable"** (case 2) — never to a
  verification failure.
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

Clean-content WAN traffic is already not forced sensitive (the 08-15
narrowing), so this adds no new relaxation — it changes a label from *claimed*
to *proven*, and adds one new active-forgery case.

CLAUDE.md's jekt section (repo copy and the workspace copies) must gain
`TRUST=wan-verified` in three places: the delivery-tier explainer, the
`TIER=info`/`coord` default list, and the `ESCALATE=none` list — plus the new
forced-sensitive bullet. Per that section's own standing rule, this documents
shipped code; it must not land ahead of the implementation.

### 3.6 Rotation — the thing LAN could not have

Because there is an authoritative registry rather than permanent pins, a
rotated WAN key is a **republish**, not a spoofing alarm. WAN keys can
therefore adopt the lazy 24h TTL rotation of
`SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md`: `agent_wan_key_ensure`
rotates at the next spawn once the row is older than the TTL, a live process
keeps signing with the key already in its env, and the republish rides the
next provision call.

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
| **W2** | Tenant-scoped injection storage: attribute + GSI, dual-write, backfill, read cutover, drop old GSI | W1 (legacy callers need a resolvable account first) | Yes — closes the CRITICAL cross-tenant queue collision |
| **W3** | WAN key material + publish/fetch + sign + verify + marker (§3.1–§3.5) | **W2** | Yes — `TRUST=wan-verified` |
| **W4** | Rotation + cache/refetch semantics (§3.6) | W3 | Hardening |

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

W3 itself (this spec's own scope) is comparable to the LAN signing PR:
one new table + migration, one common-crate signing/verification pair, four
`InjectionRequest` field changes, one marker value, one escalation branch, one
cloud attribute + route, one MCP call site, plus tests and CLAUDE.md. Call it
a few focused days.

**W0–W2 are the real cost, and W2 most of all.** A dual-write/backfill/cutover
on a live multi-tenant DynamoDB table carrying paying customers' undelivered
messages is not a code change; it is a migration with a rollback plan, a
backfill that must refuse to guess (§6.3 of the August spec: rows that cannot
be confidently attributed get flagged for manual review, never assigned to a
best-guess tenant), and a cutover window. Anyone sizing "WAN signing" should
size that, not §3.

---

## 6. Failure semantics

Identical three-way split to every other verifier here — no silent drop, no
silent trust:

- No signature → `wan_verified = None`, `TRUST=network-claimed`, tier per the
  08-15 narrowing (not forced sensitive by absence alone).
- Present, key unresolvable → `wan_verified = None`, same as above. "Nothing
  to check against" is not "check failed."
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
- New: `agentmux-srv/src/backend/storage/agent_wan_keys.rs` (mirrors `agent_lan_keys.rs`)
- `agentmux-srv/src/backend/storage/migrations.rs` — new table
- `agentmux-common/src/jekt_sign.rs` — `sign_wan_jekt`/`verify_wan_jekt`, domain-separated material
- `agentmux-srv/src/backend/reactive/types.rs` — `wan_sig`, `wan_verified`
- `agentmux-srv/src/backend/reactive/handler.rs` — escalation branch (§3.5)
- `agentmux-srv/src/backend/reactive/sanitize.rs` — `TRUST=wan-verified`
- `agentmux-srv/src/server/reactive.rs` — `verify_wan_signature`
- `agentmux-srv/src/muxbus/agent_credentials.rs` — publish pubkey on provision
- `agentmux-srv/src/muxbus/cloud_subscriber.rs` — carry sender account + `wan_sig` through delivery
- `agentmux-srv/src/server/app_api/agent_open.rs` — `AGENTMUX_WAN_KEY` env injection
- `agentmux-mcp/src/main.rs` — client-side signing
- `CLAUDE.md` + `docs/specs/ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md` — new row/values (that map's own rule: same change, not a follow-up)

**agentmux-cloud**
- `muxbus/server/src/agent-ownership.ts` — `wan_public_key` attribute, getter
- `muxbus/server/src/agent-provisioning.ts` — accept + store the published key
- `muxbus/server/src/index.ts` — `GET /agents/:agent_id/pubkey`
- `muxbus/server/src/store.ts` — sender account on the injection row (W2)

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
   for now; the receiving *instance* can log it.

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
