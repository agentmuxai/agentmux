# SPEC: WAN jekt verification — same-account agent jekts verified end to end over the cloud relay

**Date:** 2026-09-24
**Status:** proposed — nothing here is built. Measured against `agentmux`
`main` @ `d01833859` and `agentmux-cloud` `main` (server `1.8.3`, GitHub
consumer `1.4.12`), both read on 2026-09-24.
**Revision history:** revised the same day after an adversarial review found
three P1s and nine lesser defects in the first draft, all verified against
code and all accepted. The substantive changes:
- The first draft let **any agent of the account publish a key under any
  agent name**, because every spawned agent holds the account's cloud token.
  Keys are now keyed by a per-install instance id, immutable once published,
  shown in the marker, and WAN trust grants bind the instance (§2.2, §2.6,
  §4).
- Hostname-only instance labels collided across machines, and a re-minted key
  was carried before the directory held it; both would have raised false
  forgery alarms (§2.1, §2.2).
- A transient directory failure no longer maps to `Some(false)` (§2.3).
- The cloud no longer returns the sender's raw account id (§2.1).
- The HTTP verifier call site had no legitimate caller and was dropped (§2.3).
**Trigger:** Repo owner, after a WAN jekt from their own trusted agent
(`camper`) arrived `TRUST=network-claimed` with `ESCALATE=required`: *"figure
out why it wasn't verified … write the WAN verification design spec covering
both sides."*
**Scope:** agent-to-agent jekts carried by the muxbus cloud relay (delivery
tier 4, `DELIVERY=wan`), **between agents of the same AgentMux account**.
Both sides: the desktop (`agentmux-mcp`, `agentmux-srv`) and the cloud relay
(`agentmux-cloud/muxbus/server`).
**Relationship to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`:** that spec
remains the design of record for WAN signing in general — rotation and
continuity (§3.6) and the cross-account phases W0–W2. This spec:
1. **Re-measures it.** The signing half shipped after it was written (W3a),
   and several of its statements are now stale or wrong (§1.4).
2. **Adds phase W3-S, which does not wait for W2.** Inside one account the
   question "which key belongs to this sender?" can be given exactly one
   answer without tenant-scoped storage. W3-S ships carry, publish, verify
   and marker for that case only; cross-account traffic stays
   `TRUST=network-claimed` until 09-17's W2 and W3.
3. **Tightens 09-17's W3 design** where it would fail the same way:
   the instance identity (§2.2), the freshness window (§2.4), replay (§2.5),
   and trust grants (§2.6).

**Related:**
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10 M4d-6 —
  the signed `source_uid` (v2 material). Unrelated to this spec's material;
  W3-S is deliberately not gated on M4d.
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` — the LAN verifier's three-way
  result, and its trust-on-first-use pinning of a peer's key.
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
  cloud row as signed (§2.1); publish each agent's public key **per install**
  to a per-account key directory, immutable once written (§2.2); verify on the
  receiving desktop, only when sender and receiver share an account (§2.3);
  render `TRUST=wan-verified` with the sending instance (§2.6).
- **What it proves:** "agent `camper` on AgentMux install `<instance>` of
  *your own* account signed this, recently, for this recipient, once". It
  does **not** prove that name across installs: every agent holds the
  account's cloud token and can read its own machine's key store, so a
  compromised agent on any of your machines can publish a *new* instance
  claiming any name. It can never take over an existing instance's key, the
  marker names the instance, and trust grants bind it (§4).
- **Rollout needs no flag day:** every new field is optional on every hop,
  old peers ignore it, and every missing or transient piece degrades to
  today's `TRUST=network-claimed` — never to a false forgery alarm (§3).

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
  - `source_agent` is `AGENTMUX_AGENT_ID` (the stable slug) and
    `target_agent` is the `to` string as the caller typed it
    (`agentmux-mcp/src/main.rs:1335`, `:1343`). Agent ids are restricted to
    ASCII letters, digits, `_` and `-` (`sanitize.rs:124-131`).
  - `AGENTMUX_HOST_LABEL` is `registry::local_host_label()` — the lowercased
    hostname, or `"unknown"` (`registry.rs:545-550`). It is used only for WAN
    signing (`agent_config.rs:1371`, `editor_handlers.rs:1369`). Hostnames
    repeat across machines; `"unknown"` always collides.
  - Keys: `db_agent_wan_keys` (`backend/storage/agent_wan_keys.rs`, migration
    v36), one Ed25519 keypair per agent slug per srv data dir, minted by
    `agent_wan_key_ensure` and injected at spawn
    (`agent_config.rs:1312-1379`). `key_version` is always 1 and unread; a
    wiped data dir re-mints a different key at version 1 again.
    `agent_wan_public_key_load` has no production caller.
- **The srv drops the signature at the relay.** `try_cloud_relay`
  (`server/reactive.rs:1466-1550`) passes only `source`, `target_agent`,
  `message` and `priority` to `relay_inject` (`muxbus/relay.rs:88-149`),
  whose body is exactly `{target_agent, message, priority}` — pinned by a
  test (`relay.rs:283-311`). The relay also runs when the host-tier check
  failed. `sig_verified` is computed before the relay runs
  (`server/reactive.rs:1026` vs `:1294`).
- **Every spawned agent holds the account's cloud token.**
  `inject_muxbus_env` puts the logged-in user's access token into each
  agent's environment as `MUXBUS_TOKEN`
  (`server/agent_handlers/input.rs:543`, `server/muxbus_handlers.rs:339`).
  Agents also run as the same OS user as the srv and can read its data dir.
  **Nothing on a machine is out of reach of that machine's agents**, which is
  what bounds §4.

### 1.2 Cloud

- **`POST /reactive/inject`** (`muxbus/server/src/index.ts:379-485`):
  Fastify, no JSON schema; the body is destructured into a fixed field list
  (`:388`), and **any other field is silently discarded**. `source_agent`
  (from `X-Agent-ID`) and `target_agent` are trimmed and lowercased
  (`normalizeAgentId`, `:114-116`; `:384-386`, `:400`), except sources
  starting `github`. The message is stored verbatim.
- **Auth** (`auth.ts:131-203`): a PKCE user token yields `{userId: sub}`; an
  M2M token yields `{clientId, accountUserId}` — the owning human's Cognito
  `sub`, or undefined for clients provisioned before ownership existed
  (`auth.ts:121`); the legacy shared secret yields `{mode: 'legacy'}`.
  `checkAgentBinding` checks only M2M tokens, and only logs
  (`agent-binding.ts:30-58`). **The sender's account is known for most
  requests but never stored on the row.**
- **Storage** (`store.ts:296-316`): `muxbus-injections-<env>`, keyed by a
  cloud-minted `id`, GSI on bare `target_agent`. `expires_at` defaults to
  1800 s after creation (`store.ts:154-165`); the desktop relay never sets
  `ttl_seconds`.
- **Delivery:** a WebSocket wake broadcast, then
  `GET /reactive/pending/:agent_id` (gated only by `X-Agent-ID == :agent_id`,
  so any account can read any name's queue — 09-17 §2.1's W2 problem),
  `POST /reactive/ack` (atomic claim), `POST /reactive/release` on local
  failure. Unknown routes return 401 without a token and 404 with one.
- **No key directory, no public-key route, no cloud signature on agent
  traffic.** The GitHub consumer's `reagent_sig`/`reagent_key_id`/
  `reagent_msg_id`/`reagent_ts_secs` ride the row as opaque fields — **the
  precedent this spec copies for the carry.**

### 1.3 Receive side

- `cloud_subscriber::sync_agent_reactive`
  (`muxbus/cloud_subscriber.rs:690-987`) deserialises `PendingInj` (not
  `deny_unknown_fields`, so old desktops ignore new fields), builds an
  `InjectionRequest` with `delivery_tier: "wan"`, `request_id` = the cloud
  `id` and `ts_secs` unset, and calls `handler.inject_message` directly —
  bypassing `deliver()` and the HTTP path's verifiers. Only ReAgent
  verification (`:916-942`) and transcript-request tier resolution run.
- `sanitize.rs:358-370` renders WAN as `TRUST=network-claimed`; there is no
  `wan_verified` field or `wan-verified` label.
- `handler.rs:1264-1284`: `requires_stop = is_sensitive && (!verified ||
  transcript_forced)`. **An unsigned WAN jekt is not forced sensitive by
  absence alone** (the 08-15 narrowing). `camper`'s message was sensitive
  because of its content (keyword, declared tier, or a transcript request),
  and escalated because nothing could verify it.
- Trusted-peer grants: `conversation_trust_grant_check(target, requester,
  tier)` (`server/reactive.rs:742`) matches the peer **by bare name** and
  doesn't look at verification. Harmless for WAN today — an unverified
  sensitive jekt stops anyway — and unsafe the moment one can verify (§2.6).

### 1.4 Corrections to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`

| 09-17 says | Today |
|---|---|
| §1.1 "no `wan_sig` … appears anywhere" | Shipped: `wan_sig`, `wan_source_host`, `db_agent_wan_keys`, `sign_wan_jekt`/`verify_wan_jekt` with the instance-bound material §3.1 asked for. |
| §3.1 "`signed_material` carries no domain separator" | True for host/LAN; WAN material is domain-separated (`amx-jekt-wan-v1`). |
| §2.1.2 instance = `(hostname, channel)` | Hostnames collide across machines and fall back to `"unknown"` — the same last-write-wins overwrite §2.1.2 set out to prevent. The instance needs a per-install id (§2.2 here). |
| §3.2 "the trust anchor is the Cognito account boundary" | Every agent holds the account's token (`MUXBUS_TOKEN`), so an account-authenticated `PUT` lets any agent publish a key for any other; overwrite must be refused (§2.2). |
| §3.4.1 lists `wan_msg_id`, `wan_ts_secs` | Also `wan_source_agent`/`wan_target_agent`: the cloud trims and lowercases both, and MCP signs them as typed (§2.1). |
| §3.4.1 "reuse `reagent_sig_is_fresh`'s shape" | A 600 s window rejects deliveries the relay still holds for up to 1800 s (§2.4). |
| §3.4.2 verify on both entry points | The HTTP entry point's named caller (`muxbus-client`'s fallback) sends no `X-AuthKey` and gets 401 (`server/mod.rs:3055-3068`); the only other callers are local full-key agents claiming `delivery_tier: "wan"`, whose fields are all self-asserted (§2.3). |
| §4 W0 "closes cross-account impersonation at the cloud API" | It would not: `checkAgentBinding` never checks PKCE user tokens or legacy tokens, and the desktop falls back to the user token whenever per-agent provisioning fails. W0 must also bind user tokens. |

---

## 2. Design (W3-S)

### 2.1 Carry: the signed tuple survives every hop as signed

**Signed material does not change.** Every agent spawned since W3a signs
correctly already. The verifier reconstructs the material from fields carried
**as signed**, the ReAgent way:

| Field | Set by | Notes |
|---|---|---|
| `wan_sig` | MCP | base64 Ed25519 |
| `wan_msg_id` | MCP (`request_id`) | independent of the cloud row `id` |
| `wan_ts_secs` | MCP (`ts_secs`) | |
| `wan_source_agent` | MCP (`AGENTMUX_AGENT_ID`) | as signed |
| `wan_target_agent` | MCP (`to`) | as signed |
| `wan_source_host` | MCP (`AGENTMUX_HOST_LABEL`) | the instance label (§2.2) |
| `wan_source_channel` | MCP (`AGENTMUX_CHANNEL`) | local field `source_channel` |
| `sender_same_account` | **cloud, at `GET /reactive/pending` time** | `true` iff the row's stored sender account equals the *polling* caller's account. Never client-supplied. |

The cloud stores the sender's account (`userId ?? accountUserId`, absent for
legacy or unowned M2M auth) on the row but **never returns it**: the pending
queue is readable by any account that claims the name (§1.2), and the raw
Cognito `sub` is tied to a person. It returns only the comparison.

**Envelope binding.** The verifier also checks that the carried identifiers
are the ones the cloud delivered: `norm(wan_source_agent) == row.source_agent`
and `norm(wan_target_agent) == row.target_agent`, where `norm` is the cloud's
`normalizeAgentId` (trim, lowercase). Both ids pass `validate_agent_id`
(ASCII only) before carry, so ASCII lowercasing is exact. A mismatch is a
failed verification.

**Desktop send (`try_cloud_relay` → `relay_inject`).** Add the seven
client-side fields to the relay body only when **all** of these hold, and
otherwise relay exactly as today with no tuple — so every gap degrades to
`None`, never to a false alarm:
1. **The sender proved itself locally:** host-tier `sig_verified ==
   Some(true)`. A local process with the auth key can't use the relay to
   launder another agent's captured `wan_sig`. (`AGENTMUX_JEKT_KEY` and
   `AGENTMUX_WAN_KEY` are provisioned independently,
   `agent_config.rs:1322-1375`; a missing host key just means no carry.)
2. **The carried instance is this install:** `wan_source_host ==` the
   current instance label and `wan_source_channel == local_channel_id()`. An
   agent spawned before an instance-label change keeps signing the old label
   until respawned; its messages go unsigned rather than failing.
3. **The signature verifies locally** against
   `agent_wan_public_key_load(source)`.
4. **The directory is confirmed to hold exactly this key** for
   `(source, instance, channel)` — the locally recorded published key
   (§2.2). A key minted but not yet published is not carried.
5. Both ids pass `validate_agent_id`; field sizes are within the caps below.

**Cloud (`index.ts`, `store.ts`).** Accept the seven fields as optional, with
caps (signature ≤ 128 chars, identifiers ≤ 256, `wan_ts_secs` a positive
integer). An invalid field set is **dropped with a warning and the message
stored unsigned** — rejecting the request would lose the message itself
(`relay_inject` treats a 400 as a failed delivery). Store them, the sender
account, and return the seven plus `sender_same_account` from
`GET /reactive/pending/:agent_id`. The cloud never verifies.

**Desktop receive.** `PendingInj` gains the eight fields as `Option`s.
`request_id` stays the cloud `id` (ack/release depend on it); the as-signed
values live in their own `InjectionRequest` fields, read only by the
verifier.

### 2.2 Publish: per-install instances, immutable keys

**Instance identity.** Each srv data dir gets a random install id (16 hex
chars from a CSPRNG), stored in its database on first boot.
`AGENTMUX_HOST_LABEL` becomes `<hostname>~<first 8 chars of the id>` —
readable in logs, unique per install, and unchanged in format for the signer.
A wiped data dir is a **new instance**, never a changed key on an existing
one. Two channels on one machine are already separate data dirs.

**Cloud.** New table `muxbus-agent-wan-keys-<env>`: PK `account_user_id`,
SK `<agent_id>#<instance_label>#<channel>` (lowercased), attributes
`public_key`, `key_version`, `created_at`.
- `PUT /agents/:agent_id/wan-key`, body `{instance, channel, public_key,
  key_version}`. The account is the authenticated caller's, never a
  parameter; legacy auth and M2M clients without an account are rejected.
  - **A key, once written, is immutable.** A `PUT` with the same key is a
    no-op 200; a different key for an existing SK is **409** and logged with
    both fingerprints. Nothing in W3-S replaces a key; rotation arrives with
    09-17's W4 continuity chain, which can prove a replacement descends from
    the old key.
  - Because the token is held by every agent (§1.1), the `PUT` is not a
    security boundary *within* the account — immutability is. An agent can
    create a new instance entry; it cannot replace an existing one.
- `GET /agents/:agent_id/wan-key?instance=&channel=` returns the caller's own
  account's row or 404. No cross-account read exists in W3-S.
- Neither route consumes `jekt_messages` quota (09-17 §5.3); both get an
  unbilled per-account rate limit.
- Both are called by the srv with the shared user token (`load_valid_token`),
  not the per-agent M2M token, which may have no account.

**Desktop.** The srv publishes from the spawn path, where
`inject_jekt_signing_keys_into_mcp_json` already knows the slug, instance
label and channel it writes into the agent's env — so the published SK is
exactly what the signer binds. A retry loop over `db_agent_wan_keys` rows
whose published state doesn't match covers failures and offline starts, with
backoff; a 404 from an older cloud is retried at most hourly.

New columns on `db_agent_wan_keys`: `published_public_key`,
`published_instance`, `published_channel` (additive). The carry gate (§2.1
condition 4) reads them. A 409 means another key already owns this SK — only
possible if the install id was copied between machines; the srv logs it,
stops carrying for that agent, and surfaces it in the agent's diagnostics.

### 2.3 Verify: same account only, cloud subscriber path only

`verify_wan_signature(mstore, row) -> WanVerdict` runs in
`cloud_subscriber::sync_agent_reactive`, the one path real WAN deliveries
take.

**The HTTP entry point is out of W3-S.** `handle_reactive_inject` with a
client-claimed `delivery_tier: "wan"` gets `wan_verified = None` always: its
only working callers are local full-key agents, and every field they send —
including any account claim — is self-asserted. (The `muxbus-client` fallback
that was meant to use it cannot authenticate to it today, and acks after
delivering rather than claiming first; fixing it is separate work.)

| Condition | Result |
|---|---|
| no `wan_sig` | `None` |
| `sender_same_account` absent or false | `None` — cross-account, legacy, or an older cloud |
| envelope binding fails (§2.1) | `Some(false)` |
| `wan_ts_secs` outside the window (§2.4) | `None`, audit reason `wan_sig_stale` |
| directory 404 for `(agent, instance, channel)` | `None` |
| directory lookup rate-limited, 5xx, or unreachable, and no cached key | **defer**: `POST /reactive/release`, retry next cycle; after 3 deferrals or when the row's delivery window is nearly spent, deliver with `None`, audit reason `wan_key_unavailable` |
| signature fails against the key | `Some(false)` (the key is immutable, so no refetch-and-retry is needed) |
| `(instance, agent, wan_msg_id)` already seen (§2.5) | `Some(false)`, audit reason `wan_sig_replay` |
| signature verifies | `Some(true)` |

Deferring rather than failing: the LAN tier maps a rate-limited lookup to
`Some(false)` so an attacker can't exhaust a bucket to downgrade a bad
signature. Here that attack needs a same-account row (`sender_same_account`
is cloud-computed), the downgrade is to `None` — what dropping the signature
would already give — and `Some(false)` on a cloud blip would force
legitimate traffic to STOP.

**Receiver key cache.** New table `db_wan_peer_keys(agent_id, instance,
channel, public_key, fetched_at)`. Keys are immutable in W3-S, so a cached
key never expires; the cache is cleared on logout or account switch. A global
token bucket bounds directory fetches.

### 2.4 Freshness

The window must cover the relay's own delivery TTL:
`now - wan_ts_secs ≤ WAN_SIG_MAX_AGE_SECS = 1800 + 300` and
`wan_ts_secs - now ≤ 300` (clock skew between the two desktops), a constant
beside `REAGENT_SIG_MAX_AGE_SECS`, not a reuse of it. Outside the window is
`None` (stale), not `Some(false)`: a stale signature gives an attacker nothing
dropping it wouldn't, while a long `ttl_seconds` or a skewed clock is not
forgery.

### 2.5 Replay

New table `db_wan_seen_sigs(instance, agent_id, msg_id, expires_at)`, PK
`(instance, agent_id, msg_id)`:
- written **after successful local delivery**, so the relay's own
  release-and-redeliver after a failed or deferred delivery is not a replay;
- `expires_at = wan_ts_secs + WAN_SIG_MAX_AGE_SECS` — pruned only once the
  message can no longer pass freshness;
- a hit on an otherwise-valid signature is `Some(false)`. (A fleet broadcast
  signs each target with its own msgid, `agentmux-mcp/src/main.rs:1585`, so
  there is no legitimate repeat.)

Residual: two receiving installs running the same agent name keep separate
caches. Each row is claimed once, so a replay needs a *new* row, carrying
`sender_same_account` — i.e. re-injected by an agent of the same account.
Closing it fully needs a cloud conditional write on `(sender account,
wan_msg_id)`; recommended for C1 (open question 2).

### 2.6 Marker, tier rules, trust grants

- `InjectionRequest.wan_verified: Option<bool>` plus
  `wan_verified_instance: Option<String>`, both `#[serde(skip_deserializing)]`.
- `sanitize.rs`: `TRUST=wan-verified` and `FROM_INSTANCE=<instance>/<channel>`
  when `wan_verified == Some(true)`. The instance is part of the claim, not
  decoration: the name alone is only as trustworthy as the least-trusted
  machine in the account. ReAgent's `SIG=verified` is unchanged and separate.
- `handler.rs`: `Some(true)` joins `is_cryptographically_verified`;
  `Some(false)` joins the forced-sensitive set. `TRUST=wan-verified` joins the
  `ESCALATE=none` verified-sender set; transcript-request rules are
  unchanged.
- **Trust grants at tier `wan` bind the instance.** `db_conversation_trust_grants`
  gains `granted_peer_instance` (additive; part of the lookup for tier `wan`,
  empty for other tiers). A `wan` grant matches only a `Some(true)` message
  from that exact instance and channel. Existing `wan` grants have no instance
  and are **not honoured** after upgrade — the agent asks again — rather than
  guessed at. Without this, an agent on one of your machines could register a
  fresh instance named after a granted peer and have a `transcript_request`
  auto-approved.
- The audit record gains `wan_verified`, the instance and the stale, replay
  or unavailable reason.
- CLAUDE.md's jekt section gains `TRUST=wan-verified` and `FROM_INSTANCE` in
  the tier explainer, the `ESCALATE=none` list, and the forced-sensitive list
  — **in the PR that ships the verifier, not before**.

---

## 3. Rollout

Every field is optional on every hop, so order is a preference:

| Step | Repo | Ships | Old peers |
|---|---|---|---|
| C1 | cloud | stored and returned `wan_*` fields, stored sender account, `sender_same_account`; key table and routes; optional idempotency (§2.5) | old desktops never send the fields and ignore them on pending |
| D1 | agentmux | install id and instance label; publish with published-state columns; the carry and its gate (§2.1) | old cloud drops the fields → `None`; publish 404 → retried hourly |
| D2 | agentmux | verifier, peer-key cache, replay table, marker, tier rules, instance-bound `wan` grants, audit, CLAUDE.md | old senders carry nothing → `None` |

- C1's deploy is manual (`deploy.yml`, `workflow_dispatch`); check
  `/api/health` `SERVER_VERSION` before relying on it.
- D1 and D2 may ship together. Agents keep their old `AGENTMUX_HOST_LABEL`
  until respawned, and the carry gate sends them unsigned until then.
- No existing cloud row needs migrating.

## 4. Security properties and residuals

**Proves:** the message was signed by the key that *your own account*
published, immutably, for agent `A` on install `I`; it is recent, addressed
to the agent that received it, and delivered once to this install.

**Does not prove:**
- **The agent name across installs.** Every agent holds the account's cloud
  token (§1.1), so a compromised agent on any of your machines can publish a
  new instance under any name and send verified messages *from that
  instance*. It cannot touch an existing instance's key (immutability), the
  marker shows which instance spoke, and grants bind the instance. Removing
  `MUXBUS_TOKEN` from agent environments would narrow this further; that is
  a separate change with its own callers to audit (open question 4).
- **Anything against a compromised machine.** An agent can read its own
  machine's key store, as it can for the host and LAN tiers — WAN adds no new
  exposure here.
- **Anything against a compromised cloud.** The cloud serves the directory
  and can insert its own instance entries. Immutability stops it replacing
  existing keys silently only as long as the receiver has already cached the
  real key; 09-17's W4 continuity chain is the next step.
- **Cross-account identity.** Out of scope by construction (§2.3).
- **Unsigned impersonation** is unchanged: an unsigned jekt still renders
  `TRUST=network-claimed`. Binding user tokens in W0 (§1.4) is what closes
  that at the API.

## 5. Tests

**Common crate:** verification from carried fields; each carried field
altered in turn fails; envelope binding with trimmed and uppercased carried
ids; freshness boundaries (−300 s skew, 2100 s age).

**Cloud (vitest):** pass-through of the seven fields; the sender account is
taken from the token, never from the body, and never returned;
`sender_same_account` true for the same account and false for another or for
legacy; invalid fields dropped and the message still stored; `PUT` stores
under the caller's account; same key → 200; different key → 409; `GET` of
another account's key → 404; legacy and account-less M2M `PUT` rejected.

**Desktop:**
- `relay.rs`: the body-shape test gains the optional fields; each carry-gate
  condition (§2.1 1–5) failing in turn sends the old shape.
- install id minted once and stable across restarts; a fresh data dir gives
  a new instance label.
- publish from spawn; retry after failure; 409 stops carrying and is
  surfaced; 404 is not retried hot.
- verifier table (§2.3) row by row, including deferral then `None` after three
  cycles, replay → `Some(false)`, stale → `None`, foreign account → `None`,
  and the HTTP entry point always `None`.
- handler: `TRUST=wan-verified` with a keyword gives `ESCALATE=none`; a
  transcript request under `ask` still escalates; `Some(false)` forces
  sensitive.
- grants: a `wan` grant matches only its instance; a pre-upgrade `wan` grant
  does not match.

**End to end (manual, after C1 deploy):** this machine and `camper`'s, one
account, exchange jekts both ways and show `TRUST=wan-verified` with the right
`FROM_INSTANCE`. From a second account: `TRUST=network-claimed`. A tampered
relay body (debug build): `Some(false)` and a forced `TIER=sensitive`.

## 6. Open questions

1. **Stale → `None` or `Some(false)`?** Recommended `None` (§2.4).
2. **Cloud idempotency in C1?** Recommended yes: a conditional write on
   `(sender account, wan_msg_id)` closes the cross-install replay residual
   (§2.5).
3. **How does a human retire an instance** (a decommissioned machine, a
   copied data dir behind a 409)? W3-S needs at least an owner-only delete in
   the cloud, invoked from Settings, not from an agent-reachable route.
4. **Stop injecting `MUXBUS_TOKEN` into agent environments?** It would make
   the directory writable only by the srv — but first audit what uses it.

## 7. Key files

**agentmux**
- `agentmux-srv/src/server/reactive.rs` — `try_cloud_relay` (carry gate); the
  HTTP path's `wan_verified = None`
- `agentmux-srv/src/muxbus/relay.rs` — relay body
- `agentmux-srv/src/muxbus/cloud_subscriber.rs` — `PendingInj`, verifier,
  deferral via release
- `agentmux-srv/src/backend/agent_config.rs` — instance label, publish at
  spawn
- `agentmux-srv/src/backend/reactive/registry.rs` — install id
- `agentmux-srv/src/backend/storage/agent_wan_keys.rs` — published-state
  columns
- new: `backend/storage/wan_peer_keys.rs`, `wan_seen_sigs.rs`;
  `migrations.rs`; `conversation_trust_grants.rs` (instance column)
- `agentmux-srv/src/backend/reactive/types.rs`, `handler.rs`, `sanitize.rs`
- `agentmux-common/src/jekt_sign.rs` — verification from carried fields,
  `WAN_SIG_MAX_AGE_SECS`
- `CLAUDE.md` (with D2)

**agentmux-cloud**
- `muxbus/server/src/index.ts` — inject/pending fields, key routes
- `muxbus/server/src/store.ts` — row fields, sender account, optional
  idempotency
- new: `muxbus/server/src/wan-keys.ts`
- `muxbus/infrastructure/lib/constructs/muxbus-tables.ts` — key table
