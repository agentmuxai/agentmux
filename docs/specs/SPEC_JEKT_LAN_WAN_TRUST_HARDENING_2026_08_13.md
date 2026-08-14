# Spec: Securing LAN and WAN tier jekt delivery — closing cross-tenant and cross-network trust gaps

**Date:** 2026-08-13
**Type:** Security design spec (cross-repo: `agentmux`, `agentmux-cloud`, `shared-infrastructure`)
**Status:** §3 WAN P1-2 (reagent WAN signing) implemented 2026-08-14 — see
"Implementation status" below. P0-1, P0-2, P1-1, P2-1 (WAN) and all of LAN
remain proposed/not implemented. **The still-open WAN P0 findings remain
blocker-severity; do not treat this as fully closed.**
**Trigger:** User directive, after `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` shipped host-tier signing: "we need to get secure lan and wan tier, especially wan tier, it is a blocker." Then, after discovering this AgentMux instance had never completed muxbus login (root cause of "reagent's PR review jekts never arrive"): "lets get this system operational with the proper security for lan/wan, best practices... secure reagent jekt is top priority."
**Builds on:** `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` (host-tier HMAC signing, shipped), `agentmux-cloud/muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md` (the still-inert Cognito M2M binding check this spec elevates to P0), `docs/specs/SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` (prior roadmap; this spec supersedes its Phase 1/3 sequencing based on what's now confirmed live in code, not just planned).

## Implementation status (2026-08-14 addendum)

Delivered, scoped narrowly to "secure reagent jekt" per the user's explicit
priority — the GitHub review-notification consumer ("reagent") now signs
every outgoing WAN jekt with Ed25519, verified client-side by the receiving
agentmux-srv sidecar. Renders an additive `SIG=verified`/`SIG=invalid`
marker field — **`TRUST=network-claimed` and `TIER=sensitive` stay
unconditional for all WAN traffic, exactly as before; this closes "who is
this really from," not "should this auto-execute"** (§0/§4's invariant is
unchanged). Full design in §6.2; implementation:

- `agentmux-common/src/jekt_sign.rs` — `verify_reagent_jekt` + pinned
  `reagent_public_key("reagent-v1-dev")`. Ed25519 (`ed25519-dalek`), not the
  host-tier HMAC scheme, because the verifying population is every
  AgentMux instance on WAN, not one srv that minted its own symmetric key.
- `agentmux-srv`: `InjectionRequest.reagent_sig`/`reagent_key_id` (client-
  supplied) and `.reagent_verified` (server-computed, `skip_deserializing`
  — same trust boundary as host-tier's `sig_verified`). Verified in
  `cloud_subscriber::sync_agent_reactive` before delivery.
  `wrap_jekt_message` renders the new `SIG=` field.
- `agentmux-cloud`: `muxbus/consumers/github/handler.ts` signs with a
  private key from Secrets Manager (`reagent-jekt-signing-key`, PKCS8 DER
  base64) — best-effort, never blocks delivery if signing fails.
  `muxbus/server/src/store.ts`/`index.ts` pass the four `reagent_*` fields
  through as opaque storage (server never verifies).
- **Fixed two bugs found while implementing this:** (1) a double-wrap bug
  — the muxbus server pre-wrapped messages in a `[JEKT:...]` marker before
  storage, then agentmux-srv's `wrap_jekt_message` wrapped them a second
  time on delivery, nesting two marker blocks; the server no longer
  pre-wraps (Rust is now the single authoritative wrapper for every
  delivery tier). (2) `/reactive/inject` charged quota against
  `'drone_runs'` (100/month) instead of `'jekt_messages'` (2,000/month) —
  every sibling jekt-sending endpoint used the correct resource type; this
  one alone didn't, giving reagent's notification volume a 20x smaller
  ceiling than intended before a hard 402.
- **NOT deployed** — this delivers reviewable code only. Remaining manual
  steps before this is live: generate a **fresh** production Ed25519
  keypair (the `reagent-v1-dev` key committed here was generated in a
  local shell for wiring/testing and must be treated as already exposed —
  do not reuse it in Secrets Manager), store the private half as
  `reagent-jekt-signing-key` in `services/infra`, register the new public
  key under a new `key_id` in `jekt_sign.rs`, and deploy both repos.
- **Separately, and more fundamentally:** the actual reason no WAN jekt
  (reagent's or anyone else's) had ever reached this development machine
  turned out to be simpler than any of this — `db_muxbus_credentials` was
  completely empty (no `muxbus.login` had ever been completed for this
  AgentMux channel), so `cloud_subscriber` had no token to poll with at
  all. That's a per-instance operational step (an interactive PKCE browser
  login), not a code fix, and is orthogonal to everything above — reagent
  signing hardens WAN identity once delivery works, it doesn't make
  delivery work by itself.

---

## 0. TL;DR — severity-ranked findings

**WAN is the blocker, and it's worse than "identity isn't fully verified yet."** Verified directly against deployed code (not the aspirational plan docs):

1. **CRITICAL — `agent_id` is a single, flat, global namespace shared across every AgentMux account on the WAN tier, with zero tenant scoping at the storage layer.** `getPendingInjections` queries a DynamoDB GSI keyed purely on `target_agent` (`muxbus/server/src/store.ts:237-247`) — two different customers' agents named the same thing (a real, likely collision for short human-chosen names) share the same delivery queue. This is not a theoretical gap; it's how the table is actually indexed today.
2. **CRITICAL, compounds #1 — the one check that would stop this (`checkAgentBinding`) is present in every relevant route but unconditionally non-enforcing in every deployed environment** (`ENFORCE_AGENT_BINDING` is not set anywhere — confirmed via `grep` across `agentmux-cloud` and `shared-infrastructure`). It logs a mismatch and lets the request through regardless.
3. **HIGH — the legacy shared-secret auth mode bypasses identity binding entirely, by design, for any caller holding one machine-wide secret** (`auth.ts:172-176`; `checkAgentBinding` explicitly no-ops for `auth.mode !== 'cognito'`). Documented as "internal agents, migration phase," but nothing technical scopes it to internal use.
4. **Net effect, today, in production:** any WAN caller who can authenticate at all (Cognito *or* legacy) can read, inject, acknowledge, and release messages for **any `agent_id` string**, regardless of which account owns it — gated only by guessing/knowing that string. Common short agent names (`atlas`, `helper`, `aria`, `agent1`) are not a remote edge case.

**LAN is a real, separate, high-severity gap, but a design-and-priority problem rather than a live-exploited-today one:** the LAN discovery mechanism broadcasts the **same, full-access, instance-wide `X-AuthKey`** (the identical secret that gates the *entire* local `/agentmux/service` HTTP surface, not just LAN forwarding) in cleartext, to anyone who can receive an mDNS multicast packet or send a UDP probe from a private-looking source address — deliberately, per an explicit design comment accepting "LAN is trusted." That assumption doesn't hold on guest Wi-Fi, corporate networks with untrusted peers, or a compromised IoT device on the same subnet.

**What does NOT need to change:** `TIER=sensitive` for any network-tier jekt (host-tier's own trust-layer work never touched this, and this spec doesn't either) — that's the safety net regardless of how identity ends up verified. This spec is about closing *cross-tenant data exposure* and *credential exposure*, which are worse categories of problem than "a human has to confirm before acting."

---

## 1. WAN — detailed findings

### 1.1 Cross-tenant `agent_id` collision (CRITICAL)

`muxbus/server/src/store.ts:237-247` (`getPendingInjections`):
```ts
const result = await this.client.send(new QueryCommand({
    TableName: this.injectionsTable,
    IndexName: 'target_agent-created_at-index',
    KeyConditionExpression: 'target_agent = :agent',
    ...
}));
```
No account/tenant attribute anywhere in the key condition. `createInjection` (`store.ts:224-235`) writes `{ id, target_agent, source_agent, message, priority, status, created_at, ttl }` — again, no tenant field. The `/reactive/pending/:agent_id` route (`index.ts:335-359`) enforces only that the caller's *own declared* `X-Agent-ID` header matches the URL param — a self-consistency check, not proof of ownership — then defers real authorization to `checkAgentBinding`, which (§1.2) is inert.

**Concretely:** if Customer A and Customer B each independently name an agent `"atlas"`, both agents' pending messages live in the exact same DynamoDB partition, and either customer's sidecar — or anyone else who can authenticate and set `X-Agent-ID: atlas` — can read, claim, and act on the other's messages. This also means the acknowledged/claimed-first-wins semantics `cloud_subscriber.rs` relies on (the "two seats" intentional-duplicate-delivery design) offers **no protection at all** against a genuine cross-tenant collision — it was designed for the same account's two channels racing, not two different accounts colliding.

### 1.2 `checkAgentBinding` built, never enforced (CRITICAL)

`muxbus/server/src/agent-binding.ts:22-36` — already covered in depth by `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`. Restated here because it's the direct cause of §1.1's exposure being live rather than theoretical: the fix for "does this Cognito-authenticated caller actually own this agent_id" exists, is wired into all five reactive routes, and does nothing but `console.warn` in every deployed environment (`ENFORCE_AGENT_BINDING` confirmed unset via `grep -rn ENFORCE_AGENT_BINDING` across `agentmux-cloud` and `shared-infrastructure` — appears only in the check's own source and test file).

`PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`'s own status header is now stale in a way worth correcting precisely: it says Step 2 (desktop app fetching per-agent credentials) "has not shipped." It has — `agentmux-srv/src/muxbus/agent_credentials.rs` exists and is wired into `cloud_subscriber.rs`'s `sync_agent_reactive` (with fallback to the shared token on failure). This means the *precondition* for flipping `ENFORCE_AGENT_BINDING` may already be substantially met — the remaining work is verification, not new engineering (see §3.1).

### 1.3 Legacy shared-secret mode (HIGH)

`auth.ts:172-176`:
```ts
const legacyKey = await getLegacyKey(); // ONE value, from Secrets Manager `services/infra`.`muxbus-api-key`
if (legacyKey && token === legacyKey) {
    return { mode: 'legacy' };
}
```
One secret, fetched once, cached process-lifetime, shared across **every** caller using it — no per-account, per-agent, or per-purpose scoping at all. `checkAgentBinding`'s very first line (`auth?.mode !== "cognito"`) means legacy-authenticated requests **never even attempt** identity binding — they're waved through unconditionally, same as before any of this binding work existed. `getBillingTier` treats legacy as always `'metered'` (§ auth.ts:180-183) — i.e., legacy callers aren't even quota-limited the way a free-tier account would be.

The doc comment calls this "internal agents, migration phase" — a reasonable *intent*, but there is no code-level restriction (IP allowlist, separate endpoint, scoped permission) keeping it internal-only. Anyone who has ever obtained this one value (a compromised CI credential, a leaked `.env`, an insider) has unscoped, unbound, cross-tenant read/write access to the entire jekt delivery system.

### 1.4 No WAN-tier message signing (MEDIUM — mitigated once §3.1/§3.2 land, not before)

The host-tier HMAC work (`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`) is explicitly local-machine-scoped — `AGENTMUX_JEKT_KEY` never leaves the srv instance that minted it, by design. WAN messages have no equivalent: sender identity for a WAN jekt rests entirely on the auth layer (§1.2, §1.3) rather than anything cryptographically bound to the message content itself. Once §1.1/§1.2 close the tenant-isolation gap, this becomes a "harden further" item, not a standalone blocker — flagged here so it isn't lost, not because it's this spec's top priority.

### 1.5 Replay / idempotency (needs verification, not yet a confirmed finding)

`createInjection` mints `id = inj-${Date.now()}-${randomUUID().slice(0,8)}` — reasonably unpredictable. No explicit nonce/replay-window check was found in the read portion of this audit; worth a dedicated pass (can a captured, validly-authenticated `POST /reactive/inject` body be resubmitted to redeliver an old message?) before this spec is considered fully closed — tracked as an open question in §5, not blocking §3's P0 items.

---

## 2. LAN — detailed findings

### 2.1 Full-access instance secret broadcast in cleartext (HIGH)

`agentmux-srv/src/bootstrap.rs:1237-1243` confirms `LanDiscoveryController::new(..., config.auth_key.clone())` — **the exact same `auth_key`** that `server/mod.rs:1373` uses to gate the entire `/agentmux/service` and `/agentmux/reactive/*` HTTP surface (not a LAN-scoped credential). `lan_discovery.rs`:
- Publishes it as an mDNS TXT record property (`properties: [..., ("auth_key", auth_key.as_str())]`, `start()` §~198-224) — receivable by any device that can join mDNS multicast on the subnet, no authentication of the *receiver* at all.
- Also answers it over a UDP broadcast responder (`udp_responder_loop`, port `47891`) to any source whose IP address merely *looks* private/link-local (`is_lan_source`) — a check on the sender's apparent network position, not on any credential or invitation.

The code's own comment (`find_agent`, lines 589-593) states the accepted trade-off explicitly: *"LAN traffic is trusted (private network). Anyone on the LAN who can already intercept mDNS multicast can intercept the HTTP traffic too, so the key adds no exposure beyond what already exists."* This reasoning holds for **confidentiality of the key in transit** but does not account for two things that make the actual exposure worse than "no additional exposure":
- **Standing access, not one-time interception.** A passive listener who captures the key once has durable access to the *entire* local instance for as long as it keeps running — full `/agentmux/service` surface, not just the LAN-forwarding routes — not merely visibility into one forwarded message.
- **No active MITM required.** Multicast reception and this UDP responder are both purely passive/broadcast-response — an attacker doesn't need to be on-path for existing traffic, just present on the network segment at any point while discovery is active.

### 2.2 Plain HTTP peer forwarding (MEDIUM, compounds 2.1)

`LanDiscoveryController::find_agent` (`lan_discovery.rs:615`): `let peer_url = format!("http://{}:{}", peer.address, peer.port);` — no TLS. Forwarded jekt content and the `X-AuthKey` header both travel in cleartext over the LAN segment, sniffable by anyone with L2 visibility (a shared switch with port mirroring, an untrusted AP, ARP spoofing on an unsegmented network).

### 2.3 "LAN = trusted" is a weaker assumption than the code assumes (design-level)

Not a code defect, a threat-model one: modern real-world "LAN"s routinely include devices the user doesn't fully trust — guest Wi-Fi (often bridged to the main network by default on consumer APs), IoT devices with their own compromise history, shared/coworking-space networks, corporate networks under a zero-trust posture specifically *because* internal-network trust has proven unreliable. **This feature defaults to off** (`network_lan_discovery` setting, opted in via HostPopover) — that default is good and should stay — but the current design doesn't give an opted-in user a way to get the convenience without exposing the full-instance credential to the whole broadcast domain.

---

## 3. Design — prioritized fixes

### WAN

**P0-1 — Namespace `agent_id` per account at the storage layer.** The deepest, most correct fix for §1.1: injections, the agents table, and any agent-keyed lookup need a composite key (`account_user_id#agent_id` or equivalent), not a bare `agent_id`. This is a breaking schema change for a live multi-tenant table — sequence it like this codebase's own precedent for exactly this shape of migration (`SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`'s dual-write/backfill/cutover pattern): add the composite attribute + a new GSI, dual-write old and new keys, backfill existing rows, cut reads over, then drop the old GSI. Do not attempt a single-step cutover on a live table.

**P0-2 — Redesign per-agent binding around an account-scoped Cognito client + pre-token claim, THEN flip `ENFORCE_AGENT_BINDING=true`.** Originally scoped as "just verify and flip" — resolved in §5.1 to be more than that: Cognito's default 100-app-clients-per-user-pool quota means the current one-client-per-`(user, agent)` provisioning model cannot cover most real traffic at any meaningful multi-tenant scale, so flipping the flag as-built only ever protects a small, quota-limited fraction of agents. Move to one Cognito M2M client per *account* with a pre-token Lambda injecting the authorized `agent_id`(s) as a claim (§5.1 — this codebase already has the exact precedent in the `owner`-tier pre-token Lambda), THEN verify that mechanism end-to-end and flip the flag per the original plan's step 3. Also extend the desktop's existing 401-retry-with-fallback logic to cover 403 too (§5.2) before flipping, so a rollout-era false-positive mismatch degrades gracefully instead of silently stalling delivery for that agent.

**P1-1 — Scope or retire the legacy shared-secret mode.** At minimum: stop treating it as unconditionally exempt from `checkAgentBinding` — even a coarse improvement (binding legacy callers to a specific known internal-service allowlist by IP/VPC, or issuing distinct per-consumer legacy keys instead of one global value) closes most of §1.3's blast radius without waiting on every legacy caller to migrate to Cognito M2M. The original plan already scoped a full sunset as its own later phase — this spec elevates it from "someday" to "do alongside P0-1/P0-2," given §1.1's severity is compounded directly by this bypass.

**P1-2 — Extend cryptographic sender signing to the WAN tier.** Once P0-1/P0-2 close tenant isolation, add a WAN-appropriate analogue of the host-tier HMAC work — a dedicated per-`(account, agent_id)` signing secret minted alongside the account-scoped provisioning in P0-2, not derived from the Cognito client's own credential material (mixing an auth credential with a message-signing key repeats the exact mistake the host-tier spec deliberately avoided — see §6.2 for the concrete design), so `TRUST=network-claimed` jekts can eventually carry a *verified* sender claim rather than remaining purely delivery-tier-based. **`TIER=sensitive` should still apply unconditionally regardless** (§0) — this closes the "who is this really from" question, not the "should this auto-execute" one, matching the same separation of concerns `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` established for host tier.

**P2-1 — Replay/idempotency audit.** Resolve §1.5's open question with a dedicated pass over `createInjection`/`acknowledgeInjections`/`releaseInjection` before considering the WAN tier fully hardened.

### LAN

**P0-1 — Stop broadcasting the full-access instance `auth_key` over multicast/UDP.** Do not reuse `state.auth_key` for LAN discovery at all. Options, roughly in order of preference:
- Mint a **separate, narrowly-scoped LAN-forwarding credential** (only valid for the specific `/agentmux/reactive/*` forwarding routes, not the full `/agentmux/service` surface) — least-privilege, and a captured credential's blast radius shrinks to "can forward jekts to this instance," not "full local API access."
- Require an explicit **pairing step** (a short code shown in one instance's UI, typed into the other) rather than automatic broadcast trust, for users who want tighter control than "opt into LAN discovery" alone provides.
- At minimum, gate the credential behind a **challenge-response** rather than handing the raw value to any listener/prober — even a lightweight HMAC challenge keyed on the LAN-scoped credential (once P0-1's scoping lands) meaningfully raises the bar over "broadcast the plaintext secret to anyone who asks."

**P1-1 — Encrypt/authenticate peer-to-peer LAN forwarding.** Upgrade `http://` peer forwarding to something a passive LAN listener can't read — mTLS between discovered peers (using the pairing/scoped-credential material from P0-1 to bootstrap trust), or at minimum a signed-request scheme analogous to the host-tier HMAC work, so message content and any credential in flight aren't cleartext on the wire.

**P2-1 — Make the "LAN = trusted" trade-off explicit in the opt-in UI copy.** The feature already defaults off, which is right — verify the enabling toggle's copy actually communicates "this instance's access credential will be discoverable by anything else on this network," not just "enable LAN discovery," so an informed user (not the code's own assumption) is the one deciding the trust boundary.

---

## 4. What does NOT change

- `TIER=sensitive` for any LAN/WAN-delivered jekt, unconditionally — untouched by every fix above, including the WAN signing work in P1-2. Identity verification and action-authorization are different axes; this spec only strengthens the former.
- Host-tier HMAC signing (`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`) — already shipped, unaffected, and not a template to copy-paste onto WAN/LAN wholesale (different threat models, per that spec's own §3 reasoning for host tier specifically).
- The keyword-based sensitive-content escalation — orthogonal to everything in this spec, stays as-is.

---

## 5. Does flipping `ENFORCE_AGENT_BINDING` break anything? (resolved)

**Mechanically, low risk to currently-working traffic — but it also protects far less than it sounds like, and has one real operational gap.** Traced precisely through `checkAgentBinding` (`agent-binding.ts:22-36`) and every caller of it:

```ts
if (auth?.mode !== "cognito" || !auth.boundAgentId || auth.boundAgentId === claimedAgentId) {
    return false; // no mismatch — proceeds either way
}
```

Rejection (`403 agent_binding_mismatch`) requires **all three** to hold: `auth.mode === 'cognito'`, `auth.boundAgentId` is actually set, and it disagrees with the request's claimed `agent_id`. Walking every real caller:

- **Legacy shared-secret callers** (`auth.mode === 'legacy'`) — first condition true, always exempt, completely unaffected by the flag. This includes whatever currently uses `muxbus-api-key` (§1.3) — flipping the flag does **nothing** to close that gap.
- **Shared human-level PKCE-token callers** (`auth.mode === 'cognito'`, `isM2M === false`, no `clientId`/`boundAgentId` at all) — `!auth.boundAgentId` is true, exempt. This is `cloud_subscriber.rs`'s fallback path whenever a per-agent credential isn't provisioned or its fetch fails — also completely unaffected.
- **Genuinely-provisioned per-agent M2M callers, correct case** — `boundAgentId` matches the claim (both sides independently normalize via `.toLowerCase()`/`normalizeAgentId`'s `lowercase+trim`, verified consistent between `agent_credentials.rs`'s `agent_id.to_lowercase()` and `index.ts`'s `normalizeAgentId` at every relevant route, including the provisioning route itself — no case/whitespace mismatch bug found). Passes cleanly, unaffected.
- **Genuinely-provisioned per-agent M2M callers, genuine mismatch** — the only case actually rejected. No live example of this occurring was found in this audit (agent renames get a fresh provisioning under the new name, not a stale stuck binding — `agent_credential_load` is keyed by the *current* agent_id).

**So: mechanically safe to flip today, for whatever traffic already flows through the per-agent-bound path — but that traffic is likely a small minority.** See §5.1 below for why, and read §5.2 before treating "safe to flip" as "worth flipping in its current form."

### 5.1 The real blocker: per-agent Cognito app clients don't scale (RESOLVED, elevates from "open question" to a design-changing finding)

Confirmed against current AWS documentation (docs.aws.amazon.com/cognito, checked 2026-08-13): **Amazon Cognito's default quota is 100 app clients per user pool** — not the "~1000 (raisable)" this spec's first draft estimated from memory. `provisionAgentClient` (`agent-provisioning.ts`) mints one literal Cognito `UserPoolClient` per `(user_id, agent_id)` pair, in one shared pool, across the *entire* multi-tenant deployment. **100 total provisioned agents, system-wide, across every customer, is the ceiling** — not per account. Given this session's own agent roster alone spans a dozen-plus distinct names, and any real multi-customer deployment multiplies that, this model is very plausibly already near or past that ceiling, or will be shortly — well before "most agents get a bound credential" is achievable.

**Consequence:** the per-agent-Cognito-app-client mechanism, as currently built, cannot be the long-term answer regardless of whether `ENFORCE_AGENT_BINDING` is flipped — it structurally cannot cover most real traffic at any meaningful scale. Flipping the flag today would protect only whichever small fraction of agents happened to provision before hitting the quota (and would start throwing Cognito API errors for new provisioning attempts once it's hit, if it isn't already — `provisionAgentClient` has no specific handling for a Cognito-side quota rejection distinct from its own `agent_provisions` billing-quota check).

**Recommended redesign, not just a scale-up:** move to **one Cognito M2M app client per *account*** (bounded by customer count, not agent count — almost certainly well under 100 for the foreseeable future) with a **pre-token Lambda** injecting the authorized `agent_id` (or set of agent_ids) as a custom claim, based on server-side account state at token-issue time. This codebase already has exact precedent for this shape of mechanism: `SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md`'s `owner` billing tier is derived "exclusively from the Cognito-verified email in the pre-token Lambda, never from anything client-supplied" (`auth.ts`'s own doc comment on `BillingTier`). The same pattern — server-controlled claim injection at token-mint time — sidesteps the 100-client ceiling entirely, since it needs one client per account, not one per agent.

### 5.2 The operational gap worth closing before flipping regardless of §5.1

`cloud_subscriber.rs`'s `sync_agent_reactive` only treats **HTTP 401** as "credential problem, invalidate and retry with the shared-token fallback" (`resp.status() == reqwest::StatusCode::UNAUTHORIZED`). A `checkAgentBinding` rejection is **403**, which falls into the generic non-2xx branch — `tracing::warn!` and `return AgentSyncOutcome::Ok` — **no fallback retry**. If a genuine mismatch ever occurs (a bug, an edge case this audit didn't find, a future regression), that agent's WAN pull silently stops working for that poll cycle with no user-visible error, rather than degrading gracefully the way an expired/revoked credential already does.

**Recommend before flipping:** either extend the same invalidate-and-retry-with-shared-token handling to 403 responses (treats "my bound credential apparently doesn't authorize me for this agent" the same as "my credential is stale," which is a reasonable read of what a 403 here actually means operationally), or explicitly accept that a mismatch fails closed with only a debug-level log and make sure that's monitored.

## 6. Other resolved open questions

### 6.1 Replay / idempotency (§1.5) — RESOLVED

Read `acknowledgeInjections`/`releaseInjection` in full (`store.ts:272-340`):
- **Claim (`acknowledgeInjections`)** uses a genuine atomic compare-and-swap (`ConditionExpression: "#status = :pending"`); a replayed claim request for an already-claimed injection hits `ConditionalCheckFailedException` → `"Already claimed by another delivery attempt"`, not double-delivery. **Well-protected.**
- **Release** requires the exact `delivered_at` stamp from the original claim (`ConditionExpression: "#status = :delivered AND delivered_at = :claimed_at"`) — can't be replayed or hijacked by a party that doesn't hold the matching stamp. **Well-protected.**
- **Creation (`createInjection`) has no replay protection at all** — no idempotency key, no nonce; a resubmitted (captured/replayed) authenticated request mints a brand-new injection row and the message is delivered again. **Bounded severity, not a blocker**: this requires either the caller replaying their own already-authorized request (no privilege gained, just a duplicate of a message they could send again anyway) or a MITM capturing an HTTPS-transported request (mitigated by TLS being the expected transport; not independently re-verified as part of this audit — see §7 residual items). Worth an idempotency-key addition as routine hardening, not urgent relative to §5's findings.
- **Neither of these protects against §1.1's tenant-collision problem** — they prevent double-claiming/double-releasing the *same* injection row, which does nothing if two different accounts' same-named agents are sharing that row's queue in the first place. Orthogonal concerns; both need fixing.

### 6.2 WAN-tier signing key derivation (P1-2) — concrete design proposed

Do **not** derive it from the per-agent Cognito client's own secret (abandoning that model per §5.1 anyway, and reusing an auth credential as a message-signing key mixes blast radii the same way the host-tier spec explicitly avoided reusing a GitHub PAT for jekt signing). Instead: extend whatever replaces per-agent provisioning (§5.1's account-scoped-client-plus-claim design) to *also* mint a random per-`(account, agent_id)` signing secret at the same provisioning moment, stored server-side (a new DynamoDB table, or an attribute on the existing account-registry row), returned to the desktop client and used exactly like the shipped host-tier mechanism: client signs outgoing WAN jekts with it, server verifies against its own copy. This is the same `agentmux_common::jekt_sign` pattern already built and tested in `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` — relocated to a cloud-side, tenant-scoped store instead of a local-instance-scoped one. No new cryptographic design needed, just a new home for the same mechanism.

### 6.3 DynamoDB composite-key migration plan (P0-1) — concrete phases proposed

Following this codebase's own dual-write/backfill/cutover precedent (`SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`):
1. **Add the scoping attribute + new GSI.** Add `account_user_id` (or equivalent) to the injections table; add a new GSI keyed on a computed `tenant_agent_key = ${account_user_id}#${target_agent}` (or a proper composite sort key, whichever the DynamoDB access-pattern review prefers) — additive, no impact on existing reads.
2. **Dual-write.** `createInjection` writes both the legacy unscoped `target_agent` value (existing GSI keeps working) and the new scoped key, for every new row.
3. **Cut reads over — but this step depends on §1.3/P1-1 landing first, not after.** `getPendingInjections` and friends need the caller's resolved `account_user_id` to query the new scoped index — available today only for Cognito-authenticated callers (`auth.accountUserId`/`auth.userId`). A **legacy-key caller has no resolvable account at all**, so it cannot be safely scoped without deciding what account it's allowed to act as first. This is the concrete reason P0-1 and P1-1 (scope-or-retire the legacy mode) are not independently sequenceable the way the priority list in §3 might suggest read alone — legacy access needs *some* account binding (even a coarse "internal service account" assignment) before the scoped-read cutover can safely include it.
4. **Backfill existing pending rows.** Best-effort match existing unscoped rows to an owning account via whatever ownership record exists (the agents table, if it already tracks a creator/owner) — anything that can't be confidently matched should be flagged for manual review, not silently guessed at, given the cost of scoping a message to the wrong tenant.
5. **Full cutover + drop the old GSI**, once reads are confirmed exclusively using the scoped path and backfill is complete.

---

## 7. Sources

- `agentmux-cloud/muxbus/server/src/store.ts` (`createInjection`, `getPendingInjections`, lines 224-247)
- `agentmux-cloud/muxbus/server/src/index.ts` (`/reactive/pending/:agent_id`, `/reactive/ack`, `/reactive/release` route handlers, lines 335-410)
- `agentmux-cloud/muxbus/server/src/auth.ts` (`verifyRequest`, legacy key handling, lines 1-203)
- `agentmux-cloud/muxbus/server/src/agent-binding.ts` (`checkAgentBinding`)
- `agentmux-srv/src/muxbus/agent_credentials.rs` (`ensure_agent_credential` — confirms consistent lowercase normalization matching `normalizeAgentId` server-side)
- `agentmux-cloud/muxbus/server/src/agent-provisioning.ts` (`provisionAgentClient`, `clientName` — one literal Cognito app client per `(user_id, agent_id)` pair, the mechanism §5.1 finds doesn't scale)
- `agentmux-cloud/muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`
- [Quotas in Amazon Cognito](https://docs.aws.amazon.com/cognito/latest/developerguide/quotas.html) (AWS documentation, confirms the 100-app-clients-per-user-pool default quota cited in §5.1, checked 2026-08-13)
- `docs/specs/SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md` (the existing pre-token-Lambda custom-claim precedent §5.1's redesign proposal reuses)
- `agentmux-cloud/muxbus/packages/muxbus-jekt/src/index.ts` (`wrapJektMessage` — TS-side trust-label derivation, confirmed all 4 live call sites hardcode/default `deliveryTier: 'wan'`, `'host'` is currently unreachable from any cloud-side caller)
- `agentmux-srv/src/backend/lan_discovery.rs` (full file — mDNS advertisement, UDP broadcast responder, `find_agent` peer forwarding)
- `agentmux-srv/src/bootstrap.rs:1237-1243` (confirms the LAN-broadcast key is the same instance-wide `auth_key`)
- `agentmux-srv/src/server/mod.rs:1347-1373` (`X-AuthKey` middleware — what that key actually gates)
- `docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` (host-tier signing, shipped — the pattern P1-2/LAN-P1-1 extend)
- `docs/specs/SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` (prior roadmap this spec supersedes in sequencing, based on confirmed-live findings)
- `docs/specs/SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md` (the dual-write/backfill/cutover migration pattern P0-1 should follow)
