# SPEC: WAN jekt verification — same-account agent jekts verified end to end over the cloud relay

**Date:** 2026-09-24
**Status:** proposed — nothing here is built. Measured against `agentmux`
`main` @ `d01833859` and `agentmux-cloud` `main` (server `1.8.3`, GitHub
consumer `1.4.12`), both read on 2026-09-24.
**Revision history:** two adversarial reviews on 2026-09-24; every finding
was verified against code and accepted. The design changed shape twice:
- **Review 1** found that every spawned agent holds the account's cloud
  token, so an account-authenticated key directory let any agent publish a
  key under any name; that hostname labels collide; that a re-minted key was
  carried before being published; that transient lookups raised false
  alarms; that the cloud would leak the sender's account id; and that the
  HTTP verifier site had no legitimate caller.
- **Review 2** found that immutable, label-keyed directory slots could be
  claimed first by another machine, that deleting and recreating an agent
  locked it out permanently, that data dirs are per *version* (every upgrade
  would be a new instance), and that deferring via `/reactive/release` could
  lose messages.
- The result is §2.2's **self-certifying instances**: an instance id is the
  hash of an instance key that never leaves its machine, and every published
  agent key carries that instance's signature. Receivers check the chain
  themselves, so neither another agent in the account nor the cloud can
  speak as an existing instance. New instances — which anyone holding the
  account token can mint — are verified but *unapproved* until a human
  approves them, and unapproved instances get no tier relaxation (§2.6).
**Trigger:** Repo owner, after a WAN jekt from their own trusted agent
(`camper`) arrived `TRUST=network-claimed` with `ESCALATE=required`: *"figure
out why it wasn't verified … write the WAN verification design spec covering
both sides."*
**Scope:** agent-to-agent jekts carried by the muxbus cloud relay (delivery
tier 4, `DELIVERY=wan`), **between agents of the same AgentMux account**.
Both sides: the desktop (`agentmux-mcp`, `agentmux-srv`) and the cloud relay
(`agentmux-cloud/muxbus/server`).
**Relationship to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`:** that spec
remains the design of record for WAN signing in general and for the
cross-account phases W0–W2. This spec:
1. **Re-measures it** — the signing half (W3a) shipped after it was written,
   and several of its statements are now stale or wrong (§1.4).
2. **Adds phase W3-S**, same-account verification, not gated on W2.
3. **Replaces its trust anchor for W3-S.** 09-17 anchors keys in the cloud's
   account boundary and adds a continuity chain (W4) to detect cloud-side
   substitution. Self-certifying instances (§2.2) give that detection from
   the first message, and make a separate continuity chain unnecessary for
   agent-key changes within an instance.

**Related:**
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10 M4d-6 —
  the signed `source_uid` (v2 material). Unrelated to this spec's material;
  W3-S is deliberately not gated on M4d.
- `SPEC_VERSION_ISOLATION_2026_06_01.md` §5 — the per-version data dir that
  §2.2 must not tie instance identity to.
- `SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`,
  `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md` — the tier rules a
  new verified marker joins.

---

## 0. TL;DR

- **Why `camper`'s jekt wasn't verified:** its MCP *did* sign it. The
  signature never left the sending machine, no receiver can fetch the public
  key it would need, and no receiver-side verifier is wired in. Three
  separate gaps, none of them blocked on the agent-ID migration.
- **The fix, both sides:**
  - carry the signed tuple through the relay and the cloud row as signed
    (§2.1);
  - give each AgentMux install a long-lived **instance key**; publish each
    agent's key to a per-account directory **certified by that instance
    key** (§2.2);
  - verify on the receiving desktop — instance id ↔ instance key ↔
    certificate ↔ agent signature — only when sender and receiver share an
    account (§2.3);
  - render `TRUST=wan-verified` with the instance, and relax escalation
    only for instances a human has approved (§2.6).
- **What it proves:** "agent `camper` on AgentMux install `narko~1a2b3c4d` —
  whose key never left that machine — signed this, recently, for this
  recipient, once". Neither the cloud nor another agent of the account can
  forge an existing instance. Anyone with the account's token (every agent,
  today) can mint a *new* instance; it verifies, is labelled `new`, and gets
  no relaxation until a human approves it (§4).
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
    `target_agent` is the `to` string as typed
    (`agentmux-mcp/src/main.rs:1335`, `:1343`). Agent ids are ASCII letters,
    digits, `_` and `-` only (`sanitize.rs:124-131`).
  - `AGENTMUX_HOST_LABEL` is `registry::local_host_label()`: the lowercased
    hostname, read live on every call, or `"unknown"`
    (`registry.rs:545-550`). It is used only for WAN signing
    (`agent_config.rs:1371`; tests at `:1985`, `editor_handlers.rs:1369`);
    LAN takes its hostname separately (`bootstrap.rs:1545`).
  - Keys: `db_agent_wan_keys` (`backend/storage/agent_wan_keys.rs`,
    migration v36) in `objects.db`, one Ed25519 keypair per agent slug,
    minted by `agent_wan_key_ensure`, injected at spawn
    (`agent_config.rs:1312-1379`). `key_version` is always 1 and unread.
    Deleting an agent deletes its row (`storage/agents.rs:2739-2776`); a
    recreated agent of the same name gets a new key.
  - **`objects.db` is per version** for installed builds:
    `channels/<ch>/versions/<v>/data/` (`agentmux-common/src/data_paths.rs:186-203`).
    Only `channels/<ch>/config/` and `channels/<ch>/agents/` are
    channel-wide. So today every upgrade re-mints every agent's WAN key.
    Dev builds use one dir per branch; local builds get a channel per build.
- **The srv drops the signature at the relay.** `try_cloud_relay`
  (`server/reactive.rs:1466-1550`) passes only `source`, `target_agent`,
  `message` and `priority` to `relay_inject` (`muxbus/relay.rs:88-149`),
  whose body is exactly `{target_agent, message, priority}`, pinned by a test
  (`relay.rs:283-311`). The relay also runs when the host-tier check failed.
  `sig_verified` is computed before the relay runs
  (`server/reactive.rs:1026` vs `:1294`).
- **Every spawned agent holds the account's cloud token.**
  `inject_muxbus_env` puts the logged-in user's access token into each
  agent's environment as `MUXBUS_TOKEN`
  (`server/agent_handlers/input.rs:543`, `server/muxbus_handlers.rs:339`) —
  the same token the srv itself presents (`relay.rs:183`). Agents also run
  as the srv's OS user and can read its files. **The cloud cannot tell the
  srv from its agents, and nothing on a machine is out of reach of that
  machine's agents** — the two facts §2.2 and §4 are built around.

### 1.2 Cloud

- **`POST /reactive/inject`** (`muxbus/server/src/index.ts:379-485`):
  Fastify, no JSON schema; the body is destructured into a fixed field list
  (`:388`), and **any other field is silently discarded**. `source_agent`
  (from `X-Agent-ID`) and `target_agent` are trimmed and lowercased
  (`normalizeAgentId`, `:114-116`; `:384-386`, `:400`) — except sources
  starting `github`, stored raw. The message is stored verbatim.
- **Auth** (`auth.ts:131-203`): a PKCE user token yields `{userId: sub}`; an
  M2M token yields `{clientId, accountUserId}` — the owning human's `sub`,
  from the token or a registry lookup, undefined only for unowned clients;
  the legacy shared secret yields `{mode: 'legacy'}`. `checkAgentBinding`
  checks only M2M tokens, and only logs (`agent-binding.ts:30-58`). The
  sender's account is never stored on the row.
- **Storage** (`store.ts:296-316`): `muxbus-injections-<env>`, cloud-minted
  `id`, GSI on bare `target_agent`, `expires_at` 1800 s after creation by
  default (`store.ts:154-165`; the desktop relay never sets `ttl_seconds`).
- **Delivery:** a WebSocket wake broadcast, then
  `GET /reactive/pending/:agent_id` (gated only by `X-Agent-ID == :agent_id`
  — any account can read any name's queue, 09-17's W2 problem),
  `POST /reactive/ack` (atomic claim), `POST /reactive/release` on local
  failure. Release only flips the row back to pending: it sends no wake and
  doesn't extend `expires_at` (`store.ts:396-419`), and the desktop syncs
  only on a wake (`cloud_subscriber.rs:556-575`, `:673-674`).
- **No key directory and no cloud signature on agent traffic.** The GitHub
  consumer's `reagent_sig`/`reagent_key_id`/`reagent_msg_id`/
  `reagent_ts_secs` ride the row as opaque fields — **the precedent for the
  carry.**

### 1.3 Receive side

- `cloud_subscriber::sync_agent_reactive`
  (`muxbus/cloud_subscriber.rs:690-987`) deserialises `PendingInj` (not
  `deny_unknown_fields`), builds an `InjectionRequest` with
  `delivery_tier: "wan"`, `request_id` = the cloud `id` and `ts_secs` unset,
  runs ReAgent verification (`:916-942`) and
  `resolve_transcript_request_tier_fields`, and calls
  `handler.inject_message` directly — bypassing `deliver()`. The sync runs
  from the WebSocket loop (`:498`, `join_all` at `:570`).
- `sanitize.rs:358-370` renders WAN as `TRUST=network-claimed`; there is no
  `wan_verified` field or `wan-verified` label.
- `handler.rs:1264-1284`: `requires_stop = is_sensitive && (!verified ||
  transcript_forced)`. **An unsigned WAN jekt is not forced sensitive by
  absence alone** (the 08-15 narrowing). `camper`'s message was sensitive by
  content (keyword, declared tier, or transcript request) and escalated
  because nothing could verify it.
- Trusted-peer grants: `conversation_trust_grant_check(target, requester,
  tier)` (`server/reactive.rs:742`) matches the peer by bare name and ignores
  verification. `conversation_trust_grant_add` has **no production caller**
  (tests only), and the table's key is `(agent_id, granted_peer_agent_id,
  tier)` with `INSERT OR REPLACE` (`migrations.rs:1189-1195`,
  `conversation_trust_grants.rs:59-75`).

### 1.4 Corrections to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`

| 09-17 says | Today |
|---|---|
| §1.1 "no `wan_sig` … appears anywhere" | Shipped: `wan_sig`, `wan_source_host`, `db_agent_wan_keys`, `sign_wan_jekt`/`verify_wan_jekt`. |
| §3.1 "`signed_material` carries no domain separator" | True for host/LAN; WAN material is domain-separated (`amx-jekt-wan-v1`). |
| §2.1.2 instance = `(hostname, channel)` | Hostnames collide and fall back to `"unknown"`; the data dir is per version. An instance needs its own key, stored channel-wide (§2.2 here). |
| §3.2 "the trust anchor is the Cognito account boundary" | Every agent presents the same account token as the srv (§1.1), so an account-authenticated directory lets any agent publish for any name. Directory entries must be self-certifying (§2.2). |
| §3.4.1 lists `wan_msg_id`, `wan_ts_secs` | Also the as-signed source and target: the cloud normalises both (§2.1). |
| §3.4.1 "reuse `reagent_sig_is_fresh`'s shape" | A 600 s window rejects deliveries the relay still holds for 1800 s (§2.4). |
| §3.4.2 verify on both entry points | The HTTP entry point's named caller (`muxbus-client`'s fallback) sends no `X-AuthKey` and gets 401 (`server/mod.rs:3055-3068`); its other callers self-assert every field (§2.3). |
| §3.5.1 migrate existing grants | No production path creates grants yet (§1.3). |
| §4 W0 "closes cross-account impersonation at the cloud API" | `checkAgentBinding` never checks PKCE user or legacy tokens, and the desktop falls back to the user token when per-agent provisioning fails. W0 must also bind user tokens. |

---

## 2. Design (W3-S)

### 2.1 Carry: the signed tuple survives every hop as signed

**Signed material does not change.** The verifier reconstructs it from
fields carried **as signed**, the ReAgent way:

| Field | Set by | Notes |
|---|---|---|
| `wan_sig` | MCP | base64 Ed25519 |
| `wan_msg_id` | MCP (`request_id`) | independent of the cloud row `id` |
| `wan_ts_secs` | MCP (`ts_secs`) | |
| `wan_source_agent` | MCP (`AGENTMUX_AGENT_ID`) | as signed |
| `wan_target_agent` | MCP (`to`) | as signed |
| `wan_source_host` | MCP (`AGENTMUX_HOST_LABEL`) | the **instance id** (§2.2) — signed, so it binds the instance |
| `wan_source_channel` | MCP (`AGENTMUX_CHANNEL`) | local field `source_channel` |
| `wan_key_fp` | srv | fingerprint of the agent public key; **unsigned lookup hint** — a wrong value finds a key the signature won't verify under |
| `sender_same_account` | **cloud, at `GET /reactive/pending` time** | `true` iff the row's stored sender account equals the polling caller's. Never client-supplied. |

The cloud stores the sender's account (`userId ?? accountUserId`; absent for
legacy or unowned clients) on the row but **never returns it**: the pending
queue is readable by any account that claims the name (§1.2), and the raw
`sub` is tied to a person. An absent account on either side gives `false`,
logged at debug on the desktop so a misconfigured login is diagnosable.

**Envelope binding.** The carried identifiers must be the ones the cloud
delivered: `wan_source_agent` equals `row.source_agent` and
`wan_target_agent` equals `row.target_agent`, **ASCII case-insensitively
after trimming**. Ids are ASCII-only (§1.1), and this also covers
`github*` sources, which the cloud stores unnormalised. A mismatch is a
failed verification.

**Desktop send (`try_cloud_relay` → `relay_inject`).** Carry the tuple only
when **all** of these hold; otherwise relay exactly as today with no tuple,
so every gap degrades to `None`, never to a false alarm:
1. **The sender proved itself locally:** host-tier `sig_verified ==
   Some(true)`. A local process with the auth key can't launder a captured
   `wan_sig` through the relay. (`AGENTMUX_JEKT_KEY` and `AGENTMUX_WAN_KEY`
   are provisioned independently, `agent_config.rs:1322-1375`; a missing
   host key just means no carry — do not "fix" this by dropping the gate.)
2. **The carried instance and channel are this install's:**
   `wan_source_host ==` the local instance id, `wan_source_channel ==
   local_channel_id()`. An agent spawned before D1 still signs a hostname;
   its messages go unsigned until it is respawned.
3. **The signature verifies locally** against the agent's stored public key.
4. **The directory is confirmed to hold this key's certificate** (§2.2's
   published marker for this key fingerprint).
5. Both ids pass `validate_agent_id`; sizes are within the cloud's caps.

**Cloud (`index.ts`, `store.ts`).** Accept the eight client-side fields as
optional, with caps (signature ≤ 128 chars, identifiers ≤ 256, fingerprint
exactly 43 base64url chars, `wan_ts_secs` a positive integer). An invalid set
is **dropped with a warning and the message stored unsigned** — rejecting
the request would lose the message (`relay_inject` treats a 400 as a failed
delivery). Return the eight plus `sender_same_account` from
`GET /reactive/pending/:agent_id`. The cloud never verifies messages.

**Desktop receive.** `PendingInj` gains the nine fields as `Option`s.
`request_id` stays the cloud `id` (ack/release depend on it); the as-signed
values live in their own `InjectionRequest` fields, read only by the
verifier.

### 2.2 Publish: self-certifying instances

**Instance key.** On first boot, each srv channel mints an Ed25519
**instance keypair** and stores it **channel-wide**, not in the per-version
`objects.db`: a new `channels/<ch>/identity/wan.db` (dev builds: the
branch dir), file mode 0600. The **instance id** is the first 128 bits of
`SHA-256(instance public key)`, base32-encoded (26 chars). The id is not a
secret and needn't be: nobody can produce signatures that verify under it
without the private key.
- A **display label** `<hostname at mint time>~<first 8 id chars>` is
  persisted with the key and never re-read from the live hostname.
- `AGENTMUX_HOST_LABEL` carries the instance id; the signed material's
  format is unchanged.
- **Agent WAN keys move into the same channel-wide store**, imported once
  from the current version's `db_agent_wan_keys` on first boot of D1. The
  two must always live at the same scope: an instance id that survives an
  upgrade while agent keys don't would re-mint every key on every upgrade.
- Upgrades keep the instance and the keys. A new channel, a new dev branch,
  a local build channel, or a wiped channel dir is a new instance — by
  design, since each has its own key store.

**Agent-key certificate.** For each agent key, the srv signs, with the
instance key:
`amx-wan-agent-cert-v1 ␁ instance_id ␁ agent_id ␁ channel ␁ agent_pubkey ␁
display_label ␁ issued_at`.

**Cloud.** New table `muxbus-agent-wan-keys-<env>`: PK `account_user_id`,
SK `<instance_id>#<agent_id>#<channel>#<key_fp>` (content-addressed by the
agent key's fingerprint), attributes `instance_pubkey`, `agent_pubkey`,
`display_label`, `issued_at`, `cert_sig`.
- `PUT /agents/:agent_id/wan-key` with the record. The account is the
  authenticated caller's. The cloud **checks the chain** — `instance_id` is
  the hash of `instance_pubkey`, `cert_sig` verifies, `key_fp` is the
  fingerprint of `agent_pubkey` — and rejects a record that fails (400), so
  garbage never enters the directory. Receivers check it again (§2.3); the
  cloud's check is hygiene, not the trust anchor.
  - Same SK, same record → 200 no-op. Same SK, different record (only
    `display_label`/`issued_at` can differ) → accepted only with a valid
    chain, which only the instance's own srv can produce.
  - A recreated agent has a new key and therefore a new SK: no collision,
    no lockout. Old records stay valid for the old key, whose private half
    was deleted with the agent.
- `GET /agents/:agent_id/wan-key?instance=&channel=&fp=` returns the
  caller's own account's record or 404. No cross-account read in W3-S.
- Neither route consumes `jekt_messages` quota (09-17 §5.3); both get an
  unbilled per-account rate limit.
- **No delete route in W3-S.** The cloud can't tell the srv from its agents
  (§1.1), so any delete would be agent-reachable. With self-certifying
  records a delete could only deny service, never forge; it is deferred with
  open question 3.

**Desktop.** The srv publishes from the spawn path, where
`inject_jekt_signing_keys_into_mcp_json` knows the slug, instance id and
channel it writes into the agent's env, plus a retry loop over agent keys
whose certificate isn't confirmed published (backoff; a 404 from an older
cloud retried at most hourly). The store records the published fingerprint
per agent; §2.1 condition 4 reads it. Publish uses the shared user token
(`load_valid_token`), not the per-agent M2M token, which may lack an account.

### 2.3 Verify: same account, cloud subscriber path only

`verify_wan_signature(mstore, row) -> WanVerdict` runs in
`cloud_subscriber::sync_agent_reactive`, **before**
`resolve_transcript_request_tier_fields`, and not inline in the WebSocket
`select!`: the per-agent sync is spawned, and directory fetches carry a 2 s
timeout.

**The HTTP entry point is out of W3-S.** `handle_reactive_inject` with a
client-claimed `delivery_tier: "wan"` always gets `wan_verified = None`: its
only working callers are local full-key agents, and every field they send is
self-asserted.

Checks, in order:

| Condition | Result |
|---|---|
| no `wan_sig` or no `wan_key_fp` | `None` |
| `sender_same_account` absent or false | `None` — cross-account, legacy, or an older cloud |
| envelope binding fails (§2.1) | `Some(false)` |
| `wan_ts_secs` outside the window (§2.4) | `None`, audit reason `wan_sig_stale` |
| directory 404 for `(agent, instance, channel, fp)` | `None` |
| directory unreachable, rate-limited, or timed out, and no cached record | one immediate retry, then deliver with `None`, audit reason `wan_key_unavailable`. **Never** `/reactive/release`: nothing re-wakes a released row, so it could expire undelivered (§1.2). |
| record's chain fails: `instance_id ≠ hash(instance_pubkey)`, bad `cert_sig`, or `fp ≠ fingerprint(agent_pubkey)` | `Some(false)`, audit reason `wan_cert_invalid` — only a tampering cloud or a bug produces this |
| record's instance, agent, or channel ≠ the carried ones | `Some(false)` |
| message signature fails under `agent_pubkey` | `Some(false)` |
| `(instance, agent, wan_msg_id)` already seen (§2.5) | `Some(false)`, audit reason `wan_sig_replay` |
| all pass | `Some(true)`, with the instance id and display label |

Why an unavailable directory is `None`, not `Some(false)`: the LAN tier maps a
rate-limited lookup to `Some(false)` so an attacker can't exhaust a bucket to
downgrade a bad signature. Here the downgrade is to `None`, which dropping the
signature would give anyway, while `Some(false)` on a cloud blip would force
legitimate traffic to STOP.

**Receiver cache.** New table `db_wan_peer_keys(instance_id, agent_id,
channel, key_fp, instance_pubkey, agent_pubkey, display_label)`. Records are
content-addressed and self-certifying, so a cached record never expires and
needs no refetch-on-mismatch. The cache is cleared on logout or account
switch. A global token bucket bounds fetches.

### 2.4 Freshness

The window must cover the relay's own delivery TTL:
`now - wan_ts_secs ≤ WAN_SIG_MAX_AGE_SECS = 1800 + 300` and
`wan_ts_secs - now ≤ 300` (desktop clock skew). Outside is `None` (stale),
not `Some(false)`: a stale signature gives an attacker nothing that dropping
it wouldn't, while a long `ttl_seconds` or a skewed clock is not forgery.

### 2.5 Replay

New table `db_wan_seen_sigs(instance_id, agent_id, msg_id, expires_at)`,
PK `(instance_id, agent_id, msg_id)`:
- written **after successful local delivery**, so the relay's own
  release-and-redeliver after a failed local delivery is not a replay;
- `expires_at = wan_ts_secs + WAN_SIG_MAX_AGE_SECS`: pruned only once the
  message can no longer pass freshness;
- a hit on an otherwise-valid signature is `Some(false)`. (A fleet broadcast
  signs each target with its own msgid, `agentmux-mcp/src/main.rs:1585`, so
  there is no legitimate repeat.)

Residual: two receiving installs running the same agent name keep separate
caches. Each row is claimed once, so a cross-install replay needs a *new* row
with `sender_same_account`, i.e. injected by an agent of the same account.
Closing it needs a cloud conditional write on `(sender account,
wan_msg_id)` — recommended for C1 (open question 2).

### 2.6 Marker, approval, tier rules, trust grants

**Approved instances.** Anyone holding the account's token — every agent,
today — can mint a new instance and publish certified keys under any agent
name. Its messages verify, honestly: they really come from that instance. If
verification alone relaxed escalation, a compromised agent could turn a
sensitive jekt that stops today into one that doesn't. So the receiver keeps
`db_wan_known_instances(instance_id, display_label, first_seen_at,
approved_at)`:
- every verified instance is recorded on first sight;
- **approval is a human action** in Settings → Security, on an IPC command
  that is not exposed on the App API (the same boundary as other
  human-only settings);
- the receiver's own instance is approved implicitly.

**Marker.** `InjectionRequest` gains `wan_verified: Option<bool>`,
`wan_instance: Option<String>` and `wan_instance_approved: bool`, all
`#[serde(skip_deserializing)]`. `sanitize.rs` renders, for `Some(true)`:
`TRUST=wan-verified INSTANCE=<display_label>` plus `INSTANCE_STATUS=approved`
or `INSTANCE_STATUS=new`. The display label comes from the verified record
and is re-validated on receipt (`[a-z0-9.-]{1,48}~[a-z2-7]{8}`); anything else
renders as the bare instance id. ReAgent's `SIG=verified` is unchanged.

**Tier rules (`handler.rs`).**
- `Some(true)` from an **approved** instance joins
  `is_cryptographically_verified` and the `ESCALATE=none` verified-sender
  set.
- `Some(true)` from a **new** instance is treated as unverified for
  escalation — exactly today's behaviour for WAN — but still labelled
  verified, so the human sees who is asking and can approve.
- `Some(false)` joins the forced-sensitive set.
- Transcript-request rules are unchanged.

**Trust grants.** `trusted_peers` grants at tier `wan` are **not honoured**
in W3-S: `conversation_trust_grant_check` returns false for tier `wan`. No
production path creates grants today (§1.3), so nothing is lost. Honouring
them later needs the grant key to include the instance — a table-rebuild
migration, since the current primary key plus `INSERT OR REPLACE` would allow
only one instance per peer and silently rebind — and a creation path; both
are 09-17 §3.5.1's work.

**Audit** gains `wan_verified`, the instance id, the approval state, and any
stale/replay/unavailable/cert reason. **CLAUDE.md**'s jekt section gains the
new marker fields and tier rules **in the PR that ships the verifier, not
before**.

---

## 3. Rollout

Every field is optional on every hop, so the order is a preference:

| Step | Repo | Ships | Old peers |
|---|---|---|---|
| C1 | cloud | stored and returned `wan_*` fields, stored sender account, `sender_same_account`; key table and routes with chain checks; optional idempotency (§2.5) | old desktops never send the fields and ignore them on pending |
| D1 | agentmux | channel-wide identity store (instance key, imported agent keys); instance id in `AGENTMUX_HOST_LABEL`; certificates and publish; the carry and its gate | old cloud drops the fields → `None`; publish 404 → retried hourly |
| D2 | agentmux | verifier, peer cache, replay table, known-instances table and approval UI, marker, tier rules, `wan` grants off, audit, CLAUDE.md | old senders carry nothing → `None` |

- C1's deploy is manual (`deploy.yml`, `workflow_dispatch`); check
  `/api/health` `SERVER_VERSION` before relying on it.
- D1 and D2 may ship together. Agents keep their old host label until
  respawned; the carry gate sends them unsigned until then.
- No existing cloud row needs migrating.

## 4. Security properties and residuals

**Proves:** the message was signed by an agent key certified by instance
`I`'s key, which is stored only on that install; `I` published it under
*your* account; the message is recent, addressed to the agent that received
it, and delivered once to this install. With `INSTANCE_STATUS=approved`, a
human has also said `I` is one of theirs.

**Does not prove:**
- **That a new instance is legitimate.** Anyone holding the account token —
  every agent (§1.1), or a compromised cloud — can mint one. It verifies,
  shows `INSTANCE_STATUS=new`, and gets no relaxation until approved.
  Removing `MUXBUS_TOKEN` from agent environments would narrow who can mint
  one (open question 4).
- **Anything against a compromised machine.** An agent can read its own
  machine's instance key, as it can the host and LAN keys, so a compromised
  machine can speak as any agent on it. WAN adds no new exposure.
- **Cross-account identity.** Out of scope by construction (§2.3).
- **Unsigned impersonation** is unchanged: an unsigned jekt still renders
  `TRUST=network-claimed`. Binding user tokens in W0 (§1.4) closes that at
  the API.

**No longer a residual:** a compromised cloud replacing an existing
instance's keys. It can't produce a certificate under the instance key, so a
substituted record fails the chain (`Some(false)`, `wan_cert_invalid`). That
is what 09-17's W4 continuity chain was for; for agent keys within an
instance it is no longer needed. Rotating the instance key itself — not
planned — would need one.

## 5. Tests

**Common crate:** verification from carried fields; each carried field
altered in turn fails; envelope binding with trimmed, uppercased and
`github*` ids; freshness boundaries (−300 s skew, 2100 s age); certificate
chain — wrong instance id, wrong cert signature, wrong fingerprint each fail.

**Cloud (vitest):** pass-through of the eight fields; the sender account
taken from the token, never the body, never returned; `sender_same_account`
true for the same account and false for another, legacy or unowned; invalid
fields dropped and the message still stored; `PUT` rejects a broken chain;
same record → 200; a recreated agent's new key → a new record; `GET` of
another account's record → 404.

**Desktop:**
- identity store: instance key minted once, channel-wide, surviving a version
  change; agent keys imported once from `objects.db`; the display label
  doesn't follow a hostname change.
- `relay.rs`: the body-shape test gains the optional fields; each carry-gate
  condition (§2.1 1–5) failing in turn sends the old shape.
- publish from spawn; retry after failure; a 404 is not retried hot.
- the verifier table (§2.3), row by row, including a tampered directory
  record → `Some(false)`, directory timeout → `None` without a release,
  replay → `Some(false)`, stale → `None`, foreign account → `None`, and the
  HTTP entry point always `None`.
- handler: approved instance + keyword → `ESCALATE=none`; new instance +
  keyword → escalates as today; transcript request under `ask` still
  escalates; `Some(false)` forces sensitive; a `wan` grant does not match.
- the approval command is not reachable through the App API.

**End to end (manual, after C1 deploy):** this machine and `camper`'s, one
account: jekts both ways show `TRUST=wan-verified INSTANCE_STATUS=new`; after
approving, `approved`. After an upgrade, the same instance. From a second
account: `TRUST=network-claimed`. A tampered relay body or directory record
(debug build): `Some(false)` and a forced `TIER=sensitive`.

## 6. Open questions

1. **Stale → `None` or `Some(false)`?** Recommended `None` (§2.4).
2. **Cloud idempotency in C1?** Recommended yes: a conditional write on
   `(sender account, wan_msg_id)` closes the cross-install replay residual.
3. **Retiring an instance** (a decommissioned machine). A receiver can
   un-approve locally today; a directory delete needs an authentication step
   agents can't perform (step-up auth, or open question 4).
4. **Stop injecting `MUXBUS_TOKEN` into agent environments?** It would make
   the directory and instance minting srv-only; first audit what uses it.
5. **Does the per-version `objects.db` also re-mint host and LAN keys on
   every upgrade**, breaking LAN peers' pins? Out of scope here, but §1.1's
   measurement suggests it; worth checking separately.

## 7. Key files

**agentmux**
- new: `agentmux-srv/src/backend/storage/wan_identity.rs` — channel-wide
  store: instance key, agent keys, published fingerprints, certificates
- `agentmux-common/src/data_paths.rs` — the channel-wide identity dir
- `agentmux-common/src/jekt_sign.rs` — certificate sign/verify, verification
  from carried fields, `WAN_SIG_MAX_AGE_SECS`
- `agentmux-srv/src/backend/agent_config.rs` — instance id in the env,
  publish at spawn
- `agentmux-srv/src/server/reactive.rs` — `try_cloud_relay` (carry gate); the
  HTTP path's `wan_verified = None`
- `agentmux-srv/src/muxbus/relay.rs` — relay body
- `agentmux-srv/src/muxbus/cloud_subscriber.rs` — `PendingInj`, the verifier
  before transcript-request resolution
- new: `backend/storage/wan_peer_keys.rs`, `wan_seen_sigs.rs`,
  `wan_known_instances.rs`; `migrations.rs`;
  `conversation_trust_grants.rs` (tier `wan` off)
- `agentmux-srv/src/backend/reactive/types.rs`, `handler.rs`, `sanitize.rs`
- frontend: Settings → Security → WAN instances (approve / un-approve)
- `CLAUDE.md` (with D2)

**agentmux-cloud**
- `muxbus/server/src/index.ts` — inject/pending fields, key routes
- `muxbus/server/src/store.ts` — row fields, sender account, optional
  idempotency
- new: `muxbus/server/src/wan-keys.ts` — record chain check
- `muxbus/infrastructure/lib/constructs/muxbus-tables.ts` — key table
