# SPEC: WAN jekt verification — same-account agent jekts verified end to end over the cloud relay

**Date:** 2026-09-24
**Status:** proposed — nothing here is built. Measured against `agentmux`
`main` @ `d01833859` and `agentmux-cloud` `main` (server `1.8.3`, GitHub
consumer `1.4.12`), both read on 2026-09-24.
**Trigger:** Repo owner, after a WAN jekt from their own trusted agent
(`camper`) arrived `TRUST=network-claimed` with `ESCALATE=required`: *"figure
out why it wasn't verified … write the WAN verification design spec covering
both sides."*
**Scope:** agent-to-agent jekts carried by the muxbus cloud relay (delivery
tier 4, `DELIVERY=wan`), **between agents of the same AgentMux account**.
Both sides: the desktop (`agentmux-mcp`, `agentmux-srv`) and the cloud relay
(`agentmux-cloud/muxbus/server`).
**Relationship to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`:** that spec
remains the design of record for WAN signing in general — the key model,
instance-bound identity (§2.1.2), rotation and continuity (§3.6), and the
cross-account phases W0–W2. This spec does three things to it:
1. **Re-measures it.** The signing half shipped after it was written (W3a,
   plus the instance-bound material it asked for), and several of its
   statements are now stale or wrong (§1.3).
2. **Carves out a phase, W3-S, that does not wait for W2.** Verification
   between agents of the *same* account needs no tenant-scoped storage,
   because the question "which key belongs to `camper`?" has exactly one
   answer inside one account's own key directory. W3-S ships the carry,
   publish, verify, and marker parts of 09-17's W3 for that case only.
   Cross-account traffic stays `TRUST=network-claimed` until 09-17's W2 and
   W3 land.
3. **Fixes three design gaps the 09-17 spec's W3 also has:** case
   canonicalisation (§2.1), a freshness window shorter than the relay's own
   delivery TTL (§2.4), and replay (§2.5).

**Related:**
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10 M4d-6 —
  the signed `source_uid` (v2 material). Unrelated to this spec's material;
  W3-S is deliberately not gated on M4d.
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` — the LAN tier's verifier,
  whose three-way result and rate-limit-is-failure rule W3-S mirrors.
- `SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`,
  `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md` — the tier rules a
  new verified marker joins.

---

## 0. TL;DR

- **Why `camper`'s jekt wasn't verified:** its MCP *did* sign it. The
  signature never left the sending machine, no receiver can fetch the public
  key it would need, and no receiver-side verifier is wired in. Three
  separate gaps, none of them blocked on the agent-ID migration.
- **The fix, both sides:** carry the signed tuple through the relay and the
  cloud row unchanged (§2.1); publish each agent instance's public key to a
  per-account key directory in the cloud (§2.2); verify on the receiving
  desktop against that directory, and only when sender and receiver belong to
  the same account (§2.3); render `TRUST=wan-verified` (§2.6).
- **What it proves:** "this message was signed by the private key of agent
  `camper` on instance `host/channel`, as published under *your own*
  account". The trust anchor is your account's credentials plus the cloud
  directory — no stronger than that until 09-17's W4 continuity chain lands
  (§4).
- **Rollout needs no flag day:** every new field is optional on every hop,
  old peers ignore it, and a missing piece anywhere degrades to today's
  `TRUST=network-claimed`, never to a false forgery alarm (§3).

---

## 1. What happens today (measured)

### 1.1 Send side

- **MCP signs every outgoing jekt, whatever tier it will take.**
  `sign_outgoing_jekt` (`agentmux-mcp/src/main.rs:565-643`) computes
  `wan_sig` with `AGENTMUX_WAN_KEY` over
  `amx-jekt-wan-v1 ␁ msgid ␁ source_agent ␁ source_host ␁ source_channel ␁
  target_agent ␁ ts_secs ␁ message`
  (`agentmux-common/src/jekt_sign.rs:416-456`; `␁` is `U+0001`). It sends
  `wan_sig`, `wan_source_host` and `source_channel` to the local srv, with
  `request_id` = the msgid and `ts_secs` as signed
  (`agentmux-common/src/api_types.rs:276-353`).
  - `source_agent` is `AGENTMUX_AGENT_ID` and `target_agent` is the `to`
    string **as the caller typed it** — case preserved
    (`agentmux-mcp/src/main.rs:1335`, `:1343`).
  - Keys: `db_agent_wan_keys` (`agentmux-srv/src/backend/storage/agent_wan_keys.rs`,
    migration v36), one Ed25519 keypair per agent per srv instance, minted by
    `agent_wan_key_ensure` and injected at spawn with `AGENTMUX_HOST_LABEL`
    (lowercased hostname) and `AGENTMUX_CHANNEL`
    (`agentmux-srv/src/backend/agent_config.rs:1312-1379`). `key_version` is
    always 1 and unread. `agent_wan_public_key_load` has no production caller.
- **The srv drops the signature at the relay.** `try_cloud_relay`
  (`agentmux-srv/src/server/reactive.rs:1466-1550`) passes only `source`,
  `target_agent`, `message` and `priority` to `relay_inject`
  (`agentmux-srv/src/muxbus/relay.rs:88-149`), whose JSON body is exactly
  `{target_agent, message, priority}` — a test pins that shape
  (`relay.rs:283-311`). `request_id`, `ts_secs`, `wan_sig`,
  `wan_source_host` and `source_channel` never leave the machine. The relay
  also runs when the sender's host-tier check *failed*
  (`sig_verified == Some(false)`).
- **Bearer token:** the per-agent M2M token when provisioning succeeded,
  otherwise the human's PKCE user token (`relay.rs:177-190`,
  `muxbus/agent_credentials.rs`). Both resolve to the same account on the
  cloud side (§1.2).

### 1.2 Cloud

- **`POST /reactive/inject`** (`muxbus/server/src/index.ts:379-485`):
  Fastify, no JSON schema. The body is destructured into a fixed field list
  (`:388`) — **any field not on it is silently discarded**, so even a desktop
  that sent the tuple would lose it here. `source_agent` (from `X-Agent-ID`)
  and `target_agent` are **lowercased** (`:384-386`, `:400`). The message is
  stored verbatim.
- **Auth** (`auth.ts:131-203`): a PKCE user token yields `{userId: sub}`; an
  M2M token yields `{clientId, accountUserId}`, where `accountUserId` is the
  owning human's Cognito `sub`; the legacy shared secret yields
  `{mode: 'legacy'}` with no account. **The sender's account is known for
  every non-legacy request, but never stored on the row.**
- **Storage** (`store.ts:296-316`): `muxbus-injections-<env>`, keyed by a
  cloud-minted `id` (`inj-<ms>-<uuid8>`), GSI on bare `target_agent`.
  `expires_at` defaults to 1800 s after creation (`store.ts:154-165`); the
  desktop relay never sets `ttl_seconds`, other clients may (up to 86400 s).
- **Delivery:** a WebSocket wake broadcast, then
  `GET /reactive/pending/:agent_id`, `POST /reactive/ack` (atomic claim),
  `POST /reactive/release` on local failure.
- **No key directory, no public-key route, no cloud signature on agent
  traffic.** The only signed WAN traffic is the GitHub consumer's
  (`consumers/github/handler.ts`): it signs with a Secrets Manager key under
  `reagent-v1`, lowercases the target *before* signing to match the cloud's
  normalisation, and passes `reagent_sig`/`reagent_key_id`/`reagent_msg_id`/
  `reagent_ts_secs` through the row as opaque fields — **the precedent this
  spec copies for the carry.**

### 1.3 Receive side

- `cloud_subscriber::sync_agent_reactive`
  (`agentmux-srv/src/muxbus/cloud_subscriber.rs:690-987`) deserialises
  `PendingInj` (not `deny_unknown_fields`, so new fields are ignored by old
  desktops), builds an `InjectionRequest` with `delivery_tier: "wan"`,
  `request_id` = the **cloud** `id` and `ts_secs` unset, and calls
  `handler.inject_message` **directly** — bypassing `deliver()` and the HTTP
  path's verifiers. Only ReAgent verification (`:916-942`, 600 s window,
  compiled-in public key) and the transcript-request tier resolution run.
- `sanitize.rs:358-370` renders any non-host, non-channel tier without a LAN
  verification as `TRUST=network-claimed`. There is no `wan_verified` field
  and no `wan-verified` label.
- `handler.rs:1264-1284`: `requires_stop = is_sensitive && (!verified ||
  transcript_forced)`. **An unsigned WAN jekt is not forced sensitive by
  absence alone** (the 08-15 narrowing); `camper`'s message was sensitive
  because of its content (keyword match, declared tier, or a transcript
  request) and escalated because nothing could verify it. The preliminary
  draft of this file said unverified WAN traffic is always forced sensitive;
  that was wrong.
- Trusted-peer grants: `conversation_trust_grant_check(target, requester,
  tier)` (`server/reactive.rs:742`) matches the peer **by bare name**. Today
  that is harmless for WAN because an unverified sensitive jekt stops anyway.
  It stops being harmless the moment a WAN message can be verified — see
  §2.3's same-account rule.

### 1.4 Corrections to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`

| 09-17 says | Today |
|---|---|
| §1.1 "no `wan_sig` … appears anywhere" | Shipped: `wan_sig`, `wan_source_host`, `db_agent_wan_keys`, `sign_wan_jekt`/`verify_wan_jekt` with the instance-bound material 09-17 §3.1 asked for. |
| §3.1 "`signed_material` carries no domain separator" | True for host/LAN; WAN material is domain-separated (`amx-jekt-wan-v1`). |
| §3.4.1 lists `wan_msg_id`, `wan_ts_secs` | Still needed, but also `wan_source_agent` and `wan_target_agent` — the cloud lowercases both, and MCP signs them case-preserved (§2.1). |
| §3.4.1 "reuse `reagent_sig_is_fresh`'s shape" | A 600 s window rejects legitimate deliveries the relay still holds for up to 1800 s (§2.4). |
| §4 W0 "closes cross-account impersonation at the cloud API" | It would not: `checkAgentBinding` (`agent-binding.ts:30-58`) returns early for PKCE user tokens and legacy tokens, so only M2M tokens are ever checked; the desktop falls back to the user token whenever per-agent provisioning fails. W0 must also bind user tokens. |
| §7 "Replay protection … still open" | Still open cloud-side; W3-S closes it receiver-side for signed traffic (§2.5). |

---

## 2. Design (W3-S)

### 2.1 Carry: the signed tuple survives every hop unchanged

**Signed material does not change.** Every agent spawned since W3a already
signs correctly; changing the material would make them all fail. The
verifier therefore reconstructs the material from fields carried **as
signed**, the ReAgent way, instead of from the envelope the transport
rewrote:

| Field | Set by | Carried as |
|---|---|---|
| `wan_sig` | MCP | base64 Ed25519 signature |
| `wan_msg_id` | MCP (`request_id`) | the msgid as signed — independent of the cloud row `id` |
| `wan_ts_secs` | MCP (`ts_secs`) | the timestamp as signed |
| `wan_source_agent` | MCP (`AGENTMUX_AGENT_ID`) | case preserved as signed |
| `wan_target_agent` | MCP (`to`) | case preserved as signed |
| `wan_source_host` | MCP (`AGENTMUX_HOST_LABEL`) | as today |
| `wan_source_channel` | MCP (`AGENTMUX_CHANNEL`) | as signed (the local field is `source_channel`) |
| `sender_account` | **cloud, from the authenticated token** | `userId ?? accountUserId`; absent for legacy auth. Never client-supplied. |

The verifier then binds the carried values to the envelope the cloud
delivered, **case-insensitively** (identifiers are case-insensitive
everywhere else in the system, and the cloud lowercases them):
`wan_source_agent ≈ row.source_agent`, `wan_target_agent ≈ the local agent
that claimed the row`. A mismatch is a failed verification, not a skipped
one.

**Desktop send (`try_cloud_relay` → `relay_inject`).** Add the seven
client-side fields to the relay body, and only when both:
1. **the sender proved itself locally** — host-tier
   `sig_verified == Some(true)`. A request whose host check failed or was
   absent is relayed exactly as today, without the tuple; a local process
   with the auth key cannot use the relay to launder another agent's captured
   `wan_sig`.
2. **the srv's own self-check passes** — `verify_wan_jekt` against
   `agent_wan_public_key_load(source)`. A mismatch (wrong key in the env
   after a local DB reset, a bug) drops the tuple with a warning instead of
   shipping a signature every receiver would read as forgery.

**Cloud (`index.ts`, `store.ts`).** Accept the seven fields as optional
opaque strings/integers with explicit size caps (signature ≤ 128 chars,
identifiers ≤ 256, `wan_ts_secs` a positive integer), store them on the row,
store `sender_account` from `request.auth`, and return all eight from
`GET /reactive/pending/:agent_id`. The cloud never verifies, exactly as for
`reagent_*`. Oversized or malformed values reject the request with 400 — a
silent drop would hide a client bug as "unsigned".

**Desktop receive.** `PendingInj` gains the eight fields as `Option`s.
`request_id` stays the cloud `id` (ack/release depend on it); the as-signed
values live in their own fields on `InjectionRequest` and are consumed only
by the verifier.

### 2.2 Publish: a per-account key directory

**Cloud.** New table `muxbus-agent-wan-keys-<env>`: PK `account_user_id`, SK
`<agent_id>#<host>#<channel>` (all lowercased), attributes `public_key`,
`key_version`, `created_at`, `updated_at`. Instance-keyed per 09-17 §2.1.2,
so several machines and channels running the same agent name under one
account coexist instead of overwriting one another.

- `PUT /agents/:agent_id/wan-key` — body `{host, channel, public_key,
  key_version}`; the account is **the authenticated caller's**, never a
  parameter. Rejects legacy auth (no account). Rejects a `public_key` that
  isn't 32 bytes of base64. A PUT for an existing SK with a different key is
  accepted (the account owns its instances) and logged with both
  fingerprints; W4's continuity chain is what will later make that
  detectable to receivers.
- `GET /agents/:agent_id/wan-key?host=&channel=` — returns the caller's own
  account's row or 404. **No cross-account read exists in W3-S**, which also
  means the directory is not an enumeration surface.
- Neither route consumes `jekt_messages` quota (09-17 §5.3). Both get an
  unbilled per-account rate limit.

**Desktop.** The srv publishes the key of every agent it runs:
- when `cloud_subscriber` registers the agent (`add_agent`) and on each
  `sync_agent_reactive` cycle, **only if** the locally recorded
  `published_version` differs from the key's `key_version` — steady state
  is zero calls (09-17 §3.2's version-synchronised publish; provisioning is
  not the publish path);
- with the host label and channel from the same functions spawn uses
  (`registry::local_host_label`, `local_channel_id`), so the published SK is
  exactly what the signer binds;
- failure is non-fatal and retried next cycle with backoff; a 404 or 405
  from an older cloud is recorded and retried at most hourly. An unpublished
  key degrades receivers to `None`, never to `Some(false)`.

New column `published_version` on `db_agent_wan_keys` (additive migration).

### 2.3 Verify: same account only

A shared `verify_wan_signature(mstore, own_account, req) -> Option<bool>`,
called from **both** entry points: `cloud_subscriber::sync_agent_reactive`
(the ordinary path) and `handle_reactive_inject` when the request is
`delivery_tier == "wan"` (the `muxbus-client` fallback and any full-key
caller). One function, two call sites, each with a comment naming the other —
09-17 §3.4.2 records why a single-site verifier silently never runs.

`own_account` is the logged-in user's `sub` (`muxbus/pkce.rs:313`,
`creds.user_sub`).

| Condition | `wan_verified` |
|---|---|
| no `wan_sig` | `None` |
| no `sender_account`, or `sender_account ≠ own_account`, or no local login | `None` — cross-account and legacy traffic is out of W3-S scope, logged at debug |
| carried identifiers don't match the envelope (§2.1) | `Some(false)` |
| `wan_ts_secs` outside the freshness window (§2.4) | `None`, audit reason `wan_sig_stale` |
| directory returns 404 for `(agent, host, channel)` | `None` — nothing to check against |
| key lookup rate-limited or failed transiently **and** no cached key | `Some(false)` — a presented signature must not be downgraded by exhausting a bucket (LAN precedent, `server/reactive.rs` `LanPubkeyLookup::RateLimited`) |
| signature fails against the cached key | refetch once, then re-verify; still failing → `Some(false)` |
| `(key fingerprint, wan_msg_id)` already seen (§2.5) | `Some(false)`, audit reason `wan_sig_replay` |
| signature verifies | `Some(true)` |

**Why same-account only.** Two properties hold inside one account and fail
across accounts until 09-17's W2:
1. *Key resolution is unambiguous.* `(account, agent, host, channel)` names
   exactly one published key.
2. *Name-keyed trust grants stay sound.* A `trusted_peers` grant for `atlas`
   at tier `wan` can only ever be satisfied by a verified `atlas` of the
   receiver's own account — never by a stranger who registers `atlas` under
   theirs (the bypass 09-17 §3.5.1 describes). A grant then applies to that
   name on any of the owner's instances; host-granular grants remain 09-17
   §3.5.1's job.

A cross-account message whose signature would verify under the *sender's*
directory is still rendered `TRUST=network-claimed`. That is deliberately
conservative: W3-S never reports a verification it cannot resolve
unambiguously.

**Receiver key cache.** New table `db_wan_peer_keys(agent_id, host, channel,
public_key, key_version, fetched_at)` scoped to the receiver's own account
(the table is cleared on logout or account switch). TTL 1 h, one forced
refetch on a signature mismatch, global token bucket on fetches (the lookup
key is attacker-influenced). A fetched key that differs from the cached one
replaces it and logs both fingerprints; accepting the directory's answer is
W3-S's trust model (§4).

### 2.4 Freshness

The window must cover the relay's own delivery TTL, or legitimate late
deliveries read as failures: `now - wan_ts_secs ≤ WAN_SIG_MAX_AGE_SECS =
1800 + 300` and `wan_ts_secs - now ≤ 300` (clock skew between the two
desktops). A message outside the window is `None` (stale), not `Some(false)`:
an attacker gains nothing from a stale signature that dropping the signature
wouldn't give them, and a long `ttl_seconds` or a skewed clock is not
forgery. The window is a constant beside `REAGENT_SIG_MAX_AGE_SECS`, not a
reuse of it.

### 2.5 Replay

Without it, a captured signed message can be re-injected to the same target
inside the window. New table `db_wan_seen_sigs(key_fingerprint, msg_id,
seen_at)`, PK `(key_fingerprint, msg_id)`:
- a row is written **after successful local delivery**, so the relay's own
  release-and-redeliver after a failed local delivery is not a replay;
- a hit on a presented, otherwise-valid signature is `Some(false)`;
- rows older than `WAN_SIG_MAX_AGE_SECS` are pruned — anything older fails
  freshness first.

One receiving instance's cache doesn't cover another instance of the same
agent name; the atomic cloud claim delivers each row once, so a cross-instance
replay needs a *new* row, which a second instance treats as fresh. Recorded
as a residual in §4.

### 2.6 Marker and tier rules

- `InjectionRequest.wan_verified: Option<bool>`, `#[serde(skip_deserializing)]`
  like every other verification result — a client can never assert it.
- `sanitize.rs`: `TRUST=wan-verified` when `wan_verified == Some(true)` on
  tier `wan`. ReAgent's `SIG=verified` is unchanged and separate.
- `handler.rs`: `wan_verified == Some(true)` joins
  `is_cryptographically_verified`; `Some(false)` joins the forced-sensitive
  set, like a failed LAN signature. `TRUST=wan-verified` joins the
  `ESCALATE=none` verified-sender set; the transcript-request rules are
  unchanged (identity answers *who asks*, not *whether to disclose*).
- The audit record gains `wan_verified` and the `sender_account` fingerprint
  (never the raw `sub`).
- CLAUDE.md's jekt section gains `TRUST=wan-verified` in the tier explainer,
  the `ESCALATE=none` list, and the forced-sensitive list — **in the PR that
  ships the verifier, not before**.

---

## 3. Rollout

Every field is optional on every hop, so the order is a preference, not a
constraint:

| Step | Repo | Ships | Old peers |
|---|---|---|---|
| C1 | cloud | opaque `wan_*` + `sender_account` on inject/pending; key directory table and routes | old desktops never send the fields and ignore them on pending |
| D1 | agentmux | relay carries the tuple (§2.1); version-synced publish (§2.2) | old cloud drops the fields → receivers see `None`; publish gets 404 → retried hourly |
| D2 | agentmux | verifier at both call sites, peer-key cache, replay table, marker, tier rules, audit, CLAUDE.md | old senders carry nothing → `None` |

- C1's deploy is manual (`deploy.yml`, `workflow_dispatch`); check
  `/api/health` `SERVER_VERSION` before relying on it in D2's smoke test.
- D1 and D2 may ship in one release. Agents spawned before W3a have no
  `AGENTMUX_WAN_KEY` and stay `None` until respawned.
- Nothing needs a migration of existing cloud rows: rows written before C1
  simply lack the fields.

## 4. Security properties and residuals

**Proves:** the message was signed by the private key that the receiver's own
account published for `(agent, host, channel)`, within the freshness window,
addressed to the agent that received it, and not delivered before.

**Does not prove, and why that is acceptable for W3-S:**
- *Against a compromised cloud.* The cloud serves the directory; a
  compromised muxbus can publish its own key and sign as any of the
  account's agents. Anyone holding the account's credentials can do the
  same — for W3-S that is the account owner. 09-17 §3.6's continuity chain
  (W4) is what makes a substituted key detectable.
- *Cross-account identity.* Out of scope by construction (§2.3).
- *Cross-instance replay* (§2.5 residual) requires re-injecting a captured
  tuple as a new row inside the window, to another instance of the same
  receiving agent name within the same account. Bounded by the window;
  closing it needs a cloud-side idempotency key on
  `(sender_account, wan_msg_id)` — cheap to add to C1 as a conditional
  write, recommended (open question 2).
- *Unsigned impersonation* is unchanged: an unsigned jekt can still claim any
  name and still renders `TRUST=network-claimed`. Binding user tokens in W0
  (§1.4) is what closes that at the API.

## 5. Tests

**Common crate:** verification from carried fields with case differences
between carried and envelope identifiers; each carried field altered in turn
fails; freshness boundaries (−300 s skew, 2100 s age) inclusive/exclusive.

**Cloud (vitest):** pass-through of all fields to pending; `sender_account`
taken from the token and never from the body; legacy auth stores no
`sender_account`; oversized fields → 400; `PUT` stores under the caller's
account only; `GET` of another account's key → 404; legacy `PUT` → 401/403.

**Desktop:**
- `relay.rs`: the pinned body-shape test is updated to the new optional
  fields, and a request with `sig_verified != Some(true)` still sends the
  old shape.
- the self-check drops a tuple whose key doesn't match the stored public
  key.
- publish: fires once per `key_version`, retries after failure, stops
  retrying hot on 404.
- verifier table (§2.3) row by row, at **both** call sites — including
  foreign account → `None`, rate-limited with no cache → `Some(false)`,
  replay → `Some(false)`, stale → `None`, rotation with refetch → `Some(true)`.
- handler: `TRUST=wan-verified` with a keyword match gives `ESCALATE=none`;
  a transcript request under `ask` still escalates; `Some(false)` forces
  sensitive.

**End to end (manual, after C1 deploy):** two instances under one account on
different hosts — this machine and `camper`'s — exchange jekts both ways;
expect `TRUST=wan-verified`. Then from a second account expect
`TRUST=network-claimed`, and with a tampered relay body (debug build) expect
`Some(false)` and a forced `TIER=sensitive`.

## 6. Open questions

1. **Stale → `None` or `Some(false)`?** Recommended `None` (§2.4); the
   alternative flags every long-TTL or clock-skewed message as forgery.
2. **Cloud idempotency key in C1?** Recommended yes: a conditional write on
   `(sender_account, wan_msg_id)` closes the cross-instance replay residual
   for a few lines.
3. **Show the source instance in the marker** (`FROM=camper@host/channel`)?
   Useful once one name runs on several machines; no account in the marker
   either way (09-17 §9.5).

## 7. Key files

**agentmux**
- `agentmux-srv/src/server/reactive.rs` — `try_cloud_relay` (carry gate),
  `handle_reactive_inject` (second verifier call site)
- `agentmux-srv/src/muxbus/relay.rs` — relay body
- `agentmux-srv/src/muxbus/cloud_subscriber.rs` — `PendingInj`, verifier call,
  publish loop
- `agentmux-srv/src/backend/storage/agent_wan_keys.rs` — `published_version`
- new: `agentmux-srv/src/backend/storage/wan_peer_keys.rs`,
  `wan_seen_sigs.rs`; `migrations.rs`
- `agentmux-srv/src/backend/reactive/types.rs`, `handler.rs`, `sanitize.rs`
- `agentmux-common/src/jekt_sign.rs` — verification from carried fields,
  `WAN_SIG_MAX_AGE_SECS`
- `CLAUDE.md` (with D2)

**agentmux-cloud**
- `muxbus/server/src/index.ts` — inject/pending fields, key routes
- `muxbus/server/src/store.ts` — row fields, optional idempotency
- new: `muxbus/server/src/wan-keys.ts`
- `muxbus/infrastructure/lib/constructs/muxbus-tables.ts` — key table
