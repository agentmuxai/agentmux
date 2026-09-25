# SPEC: WAN jekt verification — same-account agent jekts verified end to end over the cloud relay

**Date:** 2026-09-24
**Status:** active — the pure primitives (identifiers, certificate,
revocation, envelope and freshness checks; §2.7) shipped in PR #3727; C1
shipped in agentmux-cloud#91 (merged, not yet deployed); D1a (`wan.db`,
instance key, agent keys, purge) shipped in PR #3734; D1b (certify, publish,
carry gate; `muxbus/wan_publish.rs`, `relay::wan_carry_gate`) ships in the
PR that changes this line; D2 (verifier, marker, tier rules) is not started.
Verified 2026-09-25. Measured against `agentmux`
`main` @ `d01833859` and `agentmux-cloud` `main` (server `1.8.3`, GitHub
consumer `1.4.12`), both read on 2026-09-24.
**Revision history:** three adversarial reviews on 2026-09-24; every finding
was verified against code and accepted. What each one changed:
- **Review 1:** every spawned agent holds the account's cloud token, so an
  account-authenticated key directory let any agent publish a key under any
  name. Other fixes: hostname labels collide, a re-minted key was carried
  before being published, transient lookups raised false alarms, the cloud
  would leak the sender's account id, and the HTTP verifier site had no
  legitimate caller.
- **Review 2:** label-keyed directory slots could be claimed first by another
  machine; deleting and recreating an agent locked it out; data dirs are per
  *version*; deferring via `/reactive/release` could lose messages. This
  produced **self-certifying instances** (§2.2).
- **Review 3:**
  - Instance approval needed a real boundary: agents hold `X-AuthKey`, which
    reaches every `/ws` RPC.
  - The target binding must use the polled agent, not a row field.
  - Receiver state must live in the channel-wide store, which needs
    concurrency rules.
  - The agent-delete purge must reach that store.
  - Display labels are sender-chosen.
  - A copied instance key needs revocation.
**Trigger:** Repo owner, after a WAN jekt from their own trusted agent
(`camper`) arrived `TRUST=network-claimed` with `ESCALATE=required`: *"figure
out why it wasn't verified … write the WAN verification design spec covering
both sides."*
**Scope:** agent-to-agent jekts carried by the muxbus cloud relay (delivery
tier 4, `DELIVERY=wan`) **between agents of the same AgentMux account**, and
the WAN signature path only. Both sides: the desktop (`agentmux-mcp`,
`agentmux-srv`) and the cloud relay (`agentmux-cloud/muxbus/server`).
Other verification paths on the WAN tier (ReAgent's `SIG=`) are unchanged by
this spec, and this spec's guarantees are about `TRUST=wan-verified` only.
**Relationship to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`:** that spec
remains the design of record for WAN signing in general and for the
cross-account phases W0–W2. This spec:
1. **Re-measures it:** the signing half (W3a) shipped after it was written,
   and several of its statements are now stale or wrong (§1.4).
2. **Adds phase W3-S**, same-account verification, not gated on W2.
3. **Replaces its trust anchor for W3-S.** 09-17 anchors keys in the cloud's
   account boundary and adds a continuity chain (W4) to detect cloud-side
   substitution. Self-certifying instances (§2.2) detect substitution from
   the first message.

**Related:**
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10 — M4d-1's
  purge of an agent's keys on delete (§2.2 extends it), and M4d-6's signed
  `source_uid` (unrelated material; W3-S is not gated on M4d).
- `SPEC_VERSION_ISOLATION_2026_06_01.md` §5 — the per-version data dir and
  concurrent versions of one channel.
- `SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`,
  `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md` — the tier rules a
  new verified marker joins.

---

## 0. TL;DR

- **Why `camper`'s jekt wasn't verified.** Its MCP *did* sign it, but:
  - the signature never left the sending machine;
  - no receiver can fetch the public key it would need;
  - no receiver-side verifier is wired in.

  None of these three gaps is blocked on the agent-ID migration.
- **The fix, both sides:**
  - carry the signed tuple through the relay and the cloud row as signed
    (§2.1);
  - give each AgentMux install a long-lived **instance key**, and publish
    each agent's key to a per-account directory **certified by it** (§2.2);
  - verify on the receiving desktop — instance id ↔ instance key ↔
    certificate ↔ agent signature — only when sender and receiver share an
    account (§2.3);
  - render `TRUST=wan-verified` with the instance, and relax escalation only
    for instances a human approved through a surface agents can't reach
    (§2.6).
- **What it proves:** "agent `camper` on AgentMux install
  `narko~b3kq7zfe…` signed this, recently, for this recipient, once — and
  you approved that install". Neither the cloud nor an agent elsewhere in the
  account can forge an existing instance. Anyone with the account's token
  can mint a *new* instance: it verifies, shows `new`, and gets no
  relaxation. A copied instance key is the one residual, bounded by
  revocation (§4).
- **Rollout needs no flag day:** every new field is optional on every hop,
  old peers ignore it, and every missing or transient piece degrades to
  today's `TRUST=network-claimed`, never to a false forgery alarm (§3).

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
  - `source_agent` is `AGENTMUX_AGENT_ID` (the stable slug, lowercased by
    `derive_slug`, `storage/agents.rs:231`); `target_agent` is the `to`
    string as typed (`agentmux-mcp/src/main.rs:1335`, `:1343`). Agent ids
    are ASCII letters, digits, `_` and `-` only (`sanitize.rs:124-131`).
  - `AGENTMUX_HOST_LABEL` is `registry::local_host_label()`: the lowercased
    hostname, read live on every call, or `"unknown"`
    (`registry.rs:545-550`). It is used only for WAN signing
    (`agent_config.rs:1371`; tests at `:1985`, `editor_handlers.rs:1369`);
    LAN takes its hostname separately (`bootstrap.rs:1545`).
  - Keys: `db_agent_wan_keys` (`backend/storage/agent_wan_keys.rs`,
    migration v36) in `objects.db`, one Ed25519 keypair per agent slug,
    minted by `agent_wan_key_ensure` (`INSERT OR IGNORE` + re-read) and
    injected at spawn (`agent_config.rs:1312-1379`). `key_version` is always
    1 and unread. M4d-1 deletes an agent's keys on delete and on name reuse
    (`storage/agents.rs:2738-2910`).
  - **`objects.db` is per version** for installed builds:
    `channels/<ch>/versions/<v>/data/` (`agentmux-common/src/data_paths.rs:186-203`),
    and two versions of one channel may run at once (`:176-179`). Only
    `channels/<ch>/config/` and `channels/<ch>/agents/` are channel-wide.
    Each new version starts with an empty data dir; the launcher copies only
    the old unversioned layout (`agentmux-launcher/src/data_dir.rs:131-170`).
    So today every upgrade re-mints every agent's WAN key. Dev builds use
    `dev/<branch>/[<clone_id>/]` (`data_paths.rs:1022-1030`); local builds get
    a channel per build.
- **The srv drops the signature at the relay.** `try_cloud_relay`
  (`server/reactive.rs:1466-1550`) passes only `source`, `target_agent`,
  `message` and `priority` to `relay_inject` (`muxbus/relay.rs:88-149`),
  whose body is exactly `{target_agent, message, priority}`, pinned by a
  test (`relay.rs:283-311`). The relay also runs when the host-tier check
  failed. `sig_verified` is computed before the relay runs
  (`server/reactive.rs:1026` vs `:1294`).
- **What agents hold.** Every spawned agent gets:
  - the account's cloud access token as `MUXBUS_TOKEN`
    (`server/agent_handlers/input.rs:543`, `server/muxbus_handlers.rs:339`),
    the same token the srv presents (`relay.rs:183`);
  - the srv's `AGENTMUX_AUTH_KEY` (`input.rs:564`), which reaches `/ws` and
    `/agentmux/service` (`server/mod.rs:458-460`, `auth_middleware` at
    `:2953`) — the same RPC the frontend writes settings through
    (`websocket.rs:1716`).

  Agents also run as the srv's OS user. The only srv surface they cannot
  reach is the one gated by `AGENTMUX_HOST_REG_SECRET`, held by the CEF host
  and removed from the srv's env at start (`config.rs:136-140`;
  `server/service/credential.rs`, `host_ipc.rs`). §2.2, §2.6 and §4 are
  built around these facts.

### 1.2 Cloud

- **`POST /reactive/inject`** (`muxbus/server/src/index.ts:379-485`):
  Fastify, no JSON schema; the body is destructured into a fixed field list
  (`:388`), and **any other field is silently discarded**. `source_agent`
  (from `X-Agent-ID`) and `target_agent` are trimmed and lowercased
  (`normalizeAgentId`, `:114-116`; `:384-386`, `:400`) — except sources
  starting `github`, stored raw.
- **Auth** (`auth.ts:131-203`): a PKCE user token yields `{userId: sub}`; an
  M2M token yields `{clientId, accountUserId}` — the owning human's `sub`,
  undefined only for unowned clients; the legacy shared secret yields
  `{mode: 'legacy'}`. `checkAgentBinding` checks only M2M tokens, and only
  logs (`agent-binding.ts:30-58`). The sender's account is never stored.
- **Storage** (`store.ts:296-316`): `muxbus-injections-<env>`, cloud-minted
  `id`, GSI on bare `target_agent`, `expires_at` 1800 s after creation by
  default (`store.ts:154-165`; the desktop relay never sets `ttl_seconds`).
- **Delivery:**
  - A WebSocket wake broadcast, then `GET /reactive/pending/:agent_id`. The
    only check is `X-Agent-ID == :agent_id`, so any account can read any
    name's queue (09-17's W2 problem). The response does not include
    `target_agent` (`index.ts:507-525`).
  - `POST /reactive/ack` claims the row atomically; `POST
    /reactive/release` hands it back on local failure. Release sends no wake
    and doesn't extend `expires_at` (`store.ts:396-419`), and the desktop
    syncs only on a wake (`cloud_subscriber.rs:556-575`, `:673-674`).
- **No key directory, and no cloud signature on agent traffic.** The GitHub
  consumer's `reagent_*` fields ride the row as opaque values — the
  precedent for the carry.

### 1.3 Receive side

- `cloud_subscriber::sync_agent_reactive`
  (`muxbus/cloud_subscriber.rs:690-987`) deserialises `PendingInj` (not
  `deny_unknown_fields`) and builds an `InjectionRequest` for the *polled*
  `agent_id` (`:945`), with `delivery_tier: "wan"`, `request_id` set to the
  cloud `id`, and `ts_secs` unset. It then runs ReAgent verification
  (`:916-942`) and `resolve_transcript_request_tier_fields`, and calls
  `handler.inject_message` directly, bypassing `deliver()`. The sync runs
  from the WebSocket loop (`:498`, `join_all` at `:570`). Several seats of
  one agent can pull the same queue (`:795-806`).
- `sanitize.rs:358-370` renders WAN as `TRUST=network-claimed`; there is no
  `wan_verified` field or `wan-verified` label.
- `handler.rs:1264-1284`: `requires_stop = is_sensitive && (!verified ||
  transcript_forced)`. **An unsigned WAN jekt is not forced sensitive by
  absence alone** (the 08-15 narrowing). `camper`'s message was sensitive by
  content (keyword, declared tier, or transcript request), and it escalated
  because nothing could verify it.
- **Trusted-peer grants:**
  - `conversation_trust_grant_check(target, requester, tier)`
    (`server/reactive.rs:742`) matches the peer by bare name and ignores
    verification.
  - `conversation_trust_grant_add` has no production caller (tests only).
  - The table's primary key is `(agent_id, granted_peer_agent_id, tier)`,
    written with `INSERT OR REPLACE` (`migrations.rs:1189-1195`,
    `conversation_trust_grants.rs:59-75`).

### 1.4 Corrections to `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`

| 09-17 says | Today |
|---|---|
| §1.1 "no `wan_sig` … appears anywhere" | Shipped: `wan_sig`, `wan_source_host`, `db_agent_wan_keys`, `sign_wan_jekt`/`verify_wan_jekt`. |
| §3.1 "`signed_material` carries no domain separator" | True for host/LAN; WAN material is domain-separated (`amx-jekt-wan-v1`). |
| §2.1.2 instance = `(hostname, channel)` | Hostnames collide and fall back to `"unknown"`; the data dir is per version. An instance needs its own key, stored channel-wide (§2.2 here). |
| §3.2 "the trust anchor is the Cognito account boundary" | Every agent presents the same account token as the srv (§1.1). Directory entries must be self-certifying (§2.2). |
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
| `wan_key_fp` | srv | fingerprint of the agent public key; an **unsigned lookup hint** — a wrong value finds a key the signature won't verify under |
| `sender_same_account` | **cloud, at `GET /reactive/pending` time** | `true` iff the row's stored sender account equals the polling caller's. Never client-supplied. |

The cloud stores the sender's account (`userId ?? accountUserId`; absent for
legacy or unowned clients) on the row but **never returns it**: the pending
queue is readable by any account that claims the name (§1.2), and the raw
`sub` identifies a person. If either side has no account the value is
`false`, and the desktop logs that at debug level so a misconfigured login
can be diagnosed.

**Envelope binding** — the carried identifiers must be the ones actually
delivered, compared ASCII case-insensitively after trimming (ids are
ASCII-only, §1.1; this also covers `github*` sources stored unnormalised):
- `wan_source_agent` equals the row's `source_agent`;
- `wan_target_agent` equals **the polled `agent_id`** that
  `sync_agent_reactive` is delivering to — not a row field, which the cloud
  controls and pending doesn't return. This is ReAgent's rule
  (`cloud_subscriber.rs:920-927`); it stops a cloud from moving a message
  signed for `alice` into `bob`'s queue.

A mismatch is a failed verification.

**Desktop send (`try_cloud_relay` → `relay_inject`).** Carry the tuple only
when **all** of these hold. Otherwise relay exactly as today, with no tuple,
so every gap degrades to `None`, never to a false alarm:
1. **The sender proved itself locally:** host-tier `sig_verified ==
   Some(true)`. This stops a local process holding the auth key from
   laundering a captured `wan_sig` through the relay.
   - `AGENTMUX_JEKT_KEY` and `AGENTMUX_WAN_KEY` are provisioned
     independently (`agent_config.rs:1322-1375`), and host keys stay
     per-version. After an upgrade, agents still running with an
     old-version `.mcp.json` fail this gate and send unsigned until
     relaunched.
   - Do not "fix" this by dropping the gate.
2. **The carried instance and channel are this install's:**
   `wan_source_host` equals the local instance id and `wan_source_channel`
   equals `local_channel_id()`. An agent spawned before D1 still signs a
   hostname, so its messages go unsigned until it is respawned.
3. **The signature verifies locally** against the agent's stored public key.
   A key purged by an agent delete (§2.2) fails here, so the message goes
   unsigned.
4. **The directory is confirmed to hold this key's certificate**, per
   §2.2's published marker for this fingerprint.
5. **Both ids pass `validate_agent_id`**, and sizes are within the cloud's
   caps.

**Cloud (`index.ts`, `store.ts`).**
- Accept the eight client-side fields as optional, within caps:
  - signature ≤ 128 chars;
  - identifiers ≤ 256;
  - fingerprint exactly 43 base64url chars;
  - `wan_ts_secs` a positive integer.
- An invalid set is **dropped with a warning and the message stored
  unsigned**. Rejecting the request would lose the message, because
  `relay_inject` treats a 400 as a failed delivery.
- Return the eight plus `sender_same_account` from
  `GET /reactive/pending/:agent_id`.
- **Idempotency is required, not optional:** a conditional write refuses a
  second row with the same `(sender account, wan_msg_id)`, answering 200 with
  the existing id so a retried relay isn't treated as a failure.
- The cloud never verifies messages.

**Desktop receive.** `PendingInj` gains the nine fields as `Option`s.
`request_id` stays the cloud `id`, which ack and release depend on. The
as-signed values live in their own `InjectionRequest` fields, read only by
the verifier.

### 2.2 Publish: self-certifying instances

**The channel-wide identity store.** A new SQLite file,
`<channel instance dir>/wan-identity/wan.db`, sits beside `config/` and
`agents/`. For installed and portable builds that is `channels/<ch>/`; for
dev builds it is the branch (or clone) dir. It is deliberately not named
`identity/`, which would sit next to the unrelated `identities/`. It holds
everything WAN verification needs to survive an upgrade, **on both sides**:
- the instance keypair;
- agent WAN keys and their published fingerprints;
- the receiver's peer-record cache, replay table, and known-instance
  approvals (§2.3, §2.5, §2.6).

Concurrency rules, because two versions of one channel may run at once
(`data_paths.rs:176-179`) and `SPEC_VERSION_ISOLATION` names a shared DB as a
hazard:
- WAL mode, `busy_timeout` of 5 s, file mode 0600.
- Every mint is `INSERT OR IGNORE` then re-read, as `agent_wan_key_ensure`
  already does. Two processes minting at once converge on one row.
- The schema is **additive only**. It carries a `schema_version`, and an
  older binary opening a newer file ignores unknown tables and columns. A
  binary that cannot open or migrate the file does WAN signing and
  verification **not at all** (every message `None`) and never mints into a
  fallback location.

**Instance key.**
- The first boot of D1 mints an Ed25519 instance keypair.
- The **instance id** is the first 128 bits of `SHA-256(instance public
  key)`, as lowercase, unpadded base32 (26 chars, `[a-z2-7]`). The id isn't
  secret: nobody can produce signatures that verify under it without the
  private key.
- The hostname at mint time is persisted as a **host hint** and never re-read
  from the live hostname.
- `AGENTMUX_HOST_LABEL` carries the instance id. The signed material's
  format is unchanged.

**Agent keys move into `wan.db`.**
- On the first boot of D1, agent keys are imported from that version's
  `db_agent_wan_keys`, or minted fresh if it is empty.
- From then on they live only in `wan.db`, and upgrades keep them.
- A new channel, a new dev branch, a local build channel, or a wiped channel
  dir is a new instance by design, since each has its own store.

**Agent delete (extends M4d-1).**
- The purge that deletes an agent's keys on delete or name reuse
  (`storage/agents.rs:2738-2910`) also deletes its row from `wan.db`, so a
  new agent of the same name never inherits the key.
- The "name still in use" check reads the channel-wide agent definitions,
  not only this version's `db_agents`.
- An agent still running under another version keeps its key in its env,
  but it fails carry-gate condition 3 and sends unsigned. That is not a
  false alarm.
- `storage/agents.rs` and the M4d-1 tests (`agent_tokens.rs:358-395`) are in
  D1's scope.

**Agent-key certificate.** For each agent key, the srv signs with the
instance key:
`amx-wan-agent-cert-v1 ␁ instance_id ␁ lowercase(agent_id) ␁ channel ␁
agent_pubkey ␁ host_hint ␁ issued_at`.

**Instance revocation.** An instance key can be copied off its machine by
any local agent (§4). The owner can **retire** an instance:
1. the srv mints a new instance, re-certifies its agent keys, and publishes
   them;
2. it then publishes a revocation record signed by the **old** instance key:
   `amx-wan-instance-revoke-v1 ␁ old_id ␁ new_id ␁ revoked_at`.

Revocation is sticky: nothing un-revokes an id. It is started from the same
host-gated surface as approval (§2.6).

**Cloud.** A new table, `muxbus-agent-wan-keys-<env>`:
- **PK** `account_user_id`.
- **SK** `<instance_id>#<agent_id>#<channel>#<key_fp>`, content-addressed
  by the agent key's fingerprint.
- **Attributes:** `instance_pubkey`, `agent_pubkey`, `host_hint`,
  `issued_at`, `cert_sig`.

Instance revocations go in the same table under SK `<instance_id>#revoked`.

**Cloud routes:**
- **`PUT /agents/:agent_id/wan-key`** stores a record under the
  authenticated caller's account. The cloud **checks the chain** and rejects
  a record that fails it (400):
  - `instance_id` is the hash of `instance_pubkey`;
  - `cert_sig` verifies;
  - `key_fp` is the fingerprint of `agent_pubkey`.

  This keeps garbage out of the directory. It is hygiene, not the trust
  anchor: receivers check the chain again (§2.3).
  - The same record again returns 200 and changes nothing.
  - A differing record under the same SK is accepted only with a valid
    chain, which only that instance can produce.
  - A recreated agent has a new key, so it gets a new SK. There is no
    collision and no lockout.
- **`PUT /wan-instances/:instance_id/revocation`** requires a valid
  old-key signature.
- **`GET /agents/:agent_id/wan-key?instance=&channel=&fp=`** and
  **`GET /wan-instances/:instance_id`** (returns revocation status) serve
  the caller's own account only and return 404 otherwise.
- Quota and rate limits: none of these routes consumes `jekt_messages` quota
  (09-17 §5.3), and all get an unbilled per-account rate limit.
- There is **no delete route**. The cloud can't tell the srv from its agents
  (§1.1), so a delete would be reachable by agents. A self-certifying record
  can be denied but not forged; retirement goes through revocation.

**Desktop publishing.**
- The srv publishes from the spawn path, where
  `inject_jekt_signing_keys_into_mcp_json` knows the slug, the instance id,
  and the channel it writes into the agent's env.
- A retry loop re-publishes keys whose certificate isn't confirmed
  published. It uses backoff, and a 404 from an older cloud is retried at
  most hourly.
- The store records the published fingerprint per agent; §2.1 condition 4
  reads it.
- Publishing uses the shared user token (`load_valid_token`), not the
  per-agent M2M token, which may lack an account.

### 2.3 Verify: same account, cloud subscriber path only

`verify_wan_signature(identity_store, polled_agent_id, row) -> WanVerdict`
runs in `cloud_subscriber::sync_agent_reactive`. It runs **before**
`resolve_transcript_request_tier_fields`, and not inline in the WebSocket
`select!`: the per-agent sync is spawned, and directory fetches time out
after 2 s.

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
| directory unreachable, rate-limited or timed out, and no cached record | one immediate retry, then deliver with `None`, audit reason `wan_key_unavailable`. **Never** `/reactive/release`: nothing re-wakes a released row, so it could expire undelivered (§1.2) |
| record's chain fails: `instance_id ≠ hash(instance_pubkey)`, bad `cert_sig`, or `fp ≠ fingerprint(agent_pubkey)` | `Some(false)`, audit reason `wan_cert_invalid` — only a tampering cloud or a bug produces this |
| record's instance, channel or agent (ASCII case-insensitive) ≠ the carried ones | `Some(false)` |
| message signature fails under `agent_pubkey` | `Some(false)` |
| `(instance, agent, wan_msg_id)` already seen (§2.5) | `Some(false)`, audit reason `wan_sig_replay` |
| all pass | `Some(true)`, with the instance id and host hint |

**Why an unavailable directory gives `None`, not `Some(false)`.** The LAN
tier maps a rate-limited lookup to `Some(false)`, so an attacker can't
exhaust a bucket to downgrade a bad signature. Here the downgrade would be to
`None`, which dropping the signature already gives. Meanwhile `Some(false)`
on a cloud blip would force legitimate traffic to STOP.

**Peer-record cache** (in `wan.db`):
- Records are content-addressed and self-certifying, so a cached record
  never expires and needs no refetch on mismatch.
- **Instance revocation status is not cached forever.** It is refreshed
  hourly for every approved instance, and on first sight of any instance.
- The cache is cleared on logout or account switch.
- A global token bucket bounds fetches.

### 2.4 Freshness

The window must cover the relay's own delivery TTL:
- `now - wan_ts_secs ≤ WAN_SIG_MAX_AGE_SECS = 1800 + 300`;
- `wan_ts_secs - now ≤ 300`, allowing for desktop clock skew.

A message outside the window is `None` (stale), not `Some(false)`. A stale
signature gives an attacker nothing that dropping it wouldn't, while a long
`ttl_seconds` or a skewed clock is not forgery.

### 2.5 Replay

Replay is stopped at two layers.
- **Cloud (C1, required):** the idempotency write (§2.1) refuses a second row
  for the same `(sender account, wan_msg_id)`. So a same-account agent can't
  re-inject a captured tuple as a new row, including towards a *different*
  receiving install of the same agent name.
- **Receiver:** the table `wan_seen_sigs(instance_id, agent_id, msg_id,
  expires_at)` lives in `wan.db`, so it is shared by concurrent versions and
  survives upgrades.
  - A row is written **after successful local delivery**, so the relay's
    own release-and-redeliver after a failed delivery is not a replay.
  - `expires_at = wan_ts_secs + WAN_SIG_MAX_AGE_SECS`, so a row is pruned
    only once its message can no longer pass freshness.
  - A hit on an otherwise-valid signature is `Some(false)`. A fleet
    broadcast signs each target with its own msgid
    (`agentmux-mcp/src/main.rs:1585`), so no legitimate message repeats.

**Residual:** a malicious cloud can bypass its own idempotency check. It
could then replay one captured message, within the window, to a *different*
install running the same agent name, since each install's table is local.
Bounded by the window and by the envelope binding (§2.1): same target name,
same content.

### 2.6 Marker, approval, tier rules, trust grants

**Approved instances.** Anyone holding the account's token can mint a new
instance and publish certified keys under any agent name, and today that is
every agent. Its messages verify, honestly: they really come from that
instance. If verification alone relaxed escalation, a compromised agent could
turn a sensitive jekt that stops today into one that doesn't.

So `wan.db` keeps `wan_known_instances(instance_id, host_hint,
first_seen_at, approved_at, revoked_at)`:
- Every verified instance is recorded on first sight.
- The receiver's own instance is approved implicitly.
- A revoked instance is un-approved permanently.
- **Approval is made where agents can't reach it.** An `X-AuthKey`-only RPC
  would not do: every agent holds that key (§1.1). Approval and revocation
  therefore follow the `credential_broker` pattern
  (`server/service/credential.rs`, `host_ipc.rs`):
  - a CEF-host window shows the request;
  - the srv command that records it accepts only the
    `AGENTMUX_HOST_REG_SECRET`-authenticated host channel;
  - a caller holding only `X-AuthKey` is rejected.
- A local agent can still write `wan.db` directly, as the same OS user. That
  is machine compromise (§4).
- **Amended 2026-09-24** (`SPEC_GENERIC_INTEGRATIONS_2026_09_24.md`, review
  3): until GHSA-6726-q276-g6f6 is fixed, the host-gated window and host
  channel protect against MCP tools, not against a same-user process. So
  instance approval should ship enabled only in builds that include that
  fix. Until then every instance stays `new`, and gets no relaxation.

**The approval UI and the marker never show a sender-chosen label alone.**
- The receiver renders `<host_hint>~<first 8 chars of the verified id>`.
  The suffix is derived from the id the chain proved, not taken from the
  record.
- `host_hint` is validated on receipt against `[a-z0-9.-]{1,48}`; anything
  else renders as `?`.
- The approval UI shows the full 26-character id, and warns when a new
  instance's host hint matches an approved instance's.

**Marker.**
- `InjectionRequest` gains `wan_verified: Option<bool>`,
  `wan_instance: Option<String>` and `wan_instance_status`, all
  `#[serde(skip_deserializing)]`.
- For `Some(true)`, `sanitize.rs` renders
  `TRUST=wan-verified INSTANCE=<host_hint>~<id8>` and
  `INSTANCE_STATUS=approved|new|revoked`.

**Tier rules (`handler.rs`):**
- `Some(true)` from an **approved** instance joins
  `is_cryptographically_verified` and the `ESCALATE=none` verified-sender
  set.
- `Some(true)` from a **new** instance is treated as unverified for
  escalation. That is exactly today's WAN behaviour, but the message is
  labelled, so the human sees who is asking and can approve.
- `Some(true)` from a **revoked** instance is treated like `Some(false)`.
  The key is known to be out of its owner's control.
- `Some(false)` joins the forced-sensitive set.
- Transcript-request rules are unchanged.

**Trust grants.** `trusted_peers` grants at tier `wan` are **not honoured**
in W3-S: `conversation_trust_grant_check` returns false for tier `wan`.
- Nothing is lost, because no production path creates grants today (§1.3).
- Honouring them later needs two things, both 09-17 §3.5.1's work:
  - a grant key that includes the instance. That means a table-rebuild
    migration, because the current key plus `INSERT OR REPLACE` would allow
    only one instance per peer and silently rebind.
  - a way to create grants.

**Audit and docs.**
- The audit record gains `wan_verified`, the instance id, its status, and
  any stale, replay, unavailable or cert reason.
- CLAUDE.md's jekt section gains the new marker fields and tier rules **in
  the PR that ships the verifier, not before**.

### 2.7 Wire formats (as built, 2026-09-24)

§2.2 names the certificate's fields but not their encodings. The pure half
of W3-S (`agentmux-common/src/jekt_sign.rs`, "W3-S" section) fixes them, and
the cloud's chain check must match byte for byte. Fixed vectors, computed by
an independent Node implementation, are asserted in that file's tests
(`w3s_identifiers_and_certificate_match_the_cross_language_vectors`); the
cloud's tests assert the same values.
- **Public keys** (`instance_pubkey`, `agent_pubkey`, `old_instance_pubkey`):
  standard base64 of the raw 32 bytes, padded (44 chars). The certificate
  material contains `agent_pubkey` in exactly this form.
- **Signatures** (`cert_sig`, revocation `sig`, `wan_sig`): standard base64
  of the raw 64-byte Ed25519 signature (88 chars).
- **`key_fp`:** unpadded base64url of `SHA-256(raw agent public key)`
  (43 chars).
- **Instance id:** RFC 4648 base32 alphabet, lowercased, unpadded, over the
  first 16 bytes of `SHA-256(raw instance public key)` (26 chars).
- **`issued_at`, `revoked_at`:** Unix seconds, decimal, in the material.
- **Record `agent_id`** must already be lowercase. The certificate signs the
  lowercased id, so an uppercase record id would verify while naming a
  different string; the chain check rejects it instead.
- **Record shape** (`WanKeyRecord`): `instance_id`, `agent_id`, `channel`,
  `key_fp`, `instance_pubkey`, `agent_pubkey`, `host_hint`, `issued_at`,
  `cert_sig`. Revocation (`WanRevocation`): `old_instance_id`,
  `old_instance_pubkey`, `new_instance_id`, `revoked_at`, `sig`.

The §2.3 checks are split in two so the srv can run the cheap ones before
any directory fetch: `check_wan_envelope` (envelope binding, then freshness)
and `verify_wan_against_record` (chain, record matches the carried
instance/channel/agent/fingerprint, then the message signature). The
same-account check, directory lookup and replay table are stateful and stay
in the srv (D2). `WanCheckFailure::verdict()` maps each failure to its §2.3
`None` or `Some(false)`.

---

## 3. Rollout

Every field is optional on every hop, so the order below is a preference:

| Step | Repo | Ships | Old peers |
|---|---|---|---|
| C1 | cloud | stored and returned `wan_*` fields, stored sender account, `sender_same_account`, idempotency; key and revocation routes with chain checks | old desktops never send the fields and ignore them on pending |
| D1 | agentmux | `wan.db` with its concurrency rules; instance key; agent keys imported and moved; M4d-1 purge extended; instance id in `AGENTMUX_HOST_LABEL`; certificates and publish; the carry and its gate | old cloud drops the fields → `None`; publish 404 → retried hourly |
| D2 | agentmux | verifier; peer cache and revocation refresh; replay and known-instance tables; host-gated approval/revocation window; marker, tier rules, `wan` grants off, audit, CLAUDE.md | old senders carry nothing → `None` |

- C1 is deployed by hand (`deploy.yml`, `workflow_dispatch`). Check
  `/api/health` `SERVER_VERSION` before relying on it.
- D1 and D2 may ship together.
- Agents keep their old host label until respawned, and the carry gate sends
  them unsigned until then.
- No existing cloud row needs migrating.

## 4. Security properties and residuals

**Proves:**
- The message was signed by an agent key certified by instance `I`'s key,
  and `I` published it under *your* account.
- It is recent, addressed to the agent that received it, and delivered once.
- `I` is not revoked.
- With `INSTANCE_STATUS=approved`, a human also said `I` is one of theirs,
  through a surface agents can't reach.

**Does not prove:**
- **That a new instance is legitimate.** Anyone holding the account token —
  every agent (§1.1), or a compromised cloud — can mint one. It verifies,
  shows `INSTANCE_STATUS=new`, and gets no relaxation. Removing
  `MUXBUS_TOKEN` from agent environments would narrow who can mint (open
  question 4).
- **Anything against a compromised machine.** An agent can read its own
  machine's `wan.db`, including the instance key and approvals.
  - This is **worse than the host and LAN tiers**, whose keys work only from
    that host or LAN. A copied instance key lets its holder speak as every
    agent of that instance, as an approved instance, from anywhere.
  - That lasts until the owner retires the instance (§2.2) and receivers
    refresh its status (hourly, §2.3).
  - Whether to keep the instance key out of agents' reach (an OS keychain,
    or a separate user) is open question 5.
- **Cross-account identity.** Out of scope by construction (§2.3).
- **Unsigned impersonation** is unchanged: an unsigned jekt still renders
  `TRUST=network-claimed`. Binding user tokens in W0 (§1.4) closes that at
  the API.

**No longer a residual: a compromised cloud replacing an existing
instance's keys.** It can't produce a certificate under the instance key, so
a substituted record fails the chain (`Some(false)`, `wan_cert_invalid`).
This is what 09-17's W4 continuity chain was for, and for agent keys within
an instance it is no longer needed. The cloud can still withhold records or
revocations (`None`, or delayed revocation). It cannot forge them.

## 5. Tests

**Common crate:**
- verification from carried fields, and each carried field altered in turn
  fails;
- the envelope binding with trimmed, uppercased and `github*` ids, plus a
  target that differs from the polled agent;
- freshness boundaries (−300 s skew, 2100 s age);
- the certificate chain (wrong instance id, wrong certificate signature,
  wrong fingerprint);
- a revocation signature.

**Cloud (vitest):**
- pass-through of the eight fields;
- the sender account is taken from the token, never from the body, and
  never returned;
- `sender_same_account` is true for the same account and false for another,
  legacy or unowned account;
- invalid fields are dropped and the message is still stored;
- an idempotent duplicate returns the existing id;
- `PUT` rejects a broken chain; the same record returns 200; a recreated
  agent gets a new record;
- a revocation needs an old-key signature;
- a `GET` for another account's record returns 404.

**Desktop:**
- `wan.db`:
  - two processes mint at once and converge on one instance key;
  - a binary with an older schema opens a newer file;
  - a store that won't open gives `None` everywhere and never mints
    elsewhere;
  - agent keys are imported once;
  - the instance survives a version change;
  - the host hint doesn't follow a hostname change.
- **Purge:** deleting an agent removes its `wan.db` row, and a new agent of
  the same name gets a new key.
- **`relay.rs`:** the body-shape test gains the optional fields, and each
  carry-gate condition (§2.1 1–5) failing in turn sends the old shape.
- **Publish:**
  - publish happens from spawn;
  - a failure is retried;
  - a 404 is not retried hot;
  - retirement publishes a new instance, then the revocation.
- **Verifier** — the §2.3 table row by row, including:
  - a tampered directory record gives `Some(false)`;
  - a directory timeout gives `None` without a release;
  - a replay gives `Some(false)`;
  - a stale message gives `None`;
  - a foreign account gives `None`;
  - the HTTP entry point always gives `None`.
- **Handler:**
  - approved instance with a keyword → `ESCALATE=none`;
  - new instance with a keyword → escalates as today;
  - revoked instance → forced sensitive;
  - a transcript request under `ask` still escalates;
  - `Some(false)` forces sensitive;
  - a `wan` grant does not match.
- **Approval boundary:** approval and revocation reject a caller holding
  only `X-AuthKey`.

**End to end (manual, after the C1 deploy).** Run this machine and
`camper`'s under one account:
- Jekts both ways show `TRUST=wan-verified INSTANCE_STATUS=new`, and
  `approved` after approval in the host window.
- After an upgrade, it is still the same instance.
- After retiring an instance, the old id shows `revoked` within an hour.
- From a second account, jekts show `TRUST=network-claimed`.
- A tampered relay body or directory record (debug build) gives
  `Some(false)` and forces `TIER=sensitive`.

## 6. Open questions

1. **Stale → `None` or `Some(false)`?** Recommended: `None` (§2.4).
2. **Revocation refresh interval.** Hourly bounds a copied key's useful life
   after retirement; shorter costs more directory calls.
3. **Retiring an instance whose machine is gone.** Revocation needs the old
   key. If the machine is lost, only receivers' local un-approval is left.
   An account-level revocation would need step-up authentication that agents
   can't perform.
4. **Stop injecting `MUXBUS_TOKEN` into agent environments?** It would make
   the directory and instance minting srv-only; first audit what uses it.
5. **Keep the instance key out of agents' reach** (OS keychain, a separate
   user)? It would remove the §4 copied-key residual.
6. **Does the per-version `objects.db` also re-mint host and LAN keys on
   every upgrade**, breaking LAN peers' pins? Out of scope here, but §1.1's
   measurement suggests so; worth checking separately.

## 7. Key files

**agentmux**
- new: `agentmux-srv/src/backend/storage/wan_identity.rs` — the `wan.db`
  store: instance key, agent keys, published fingerprints, certificates,
  peer cache, replay table, known instances
- `agentmux-common/src/data_paths.rs` — the channel-wide `wan-identity/` dir
- `agentmux-common/src/jekt_sign.rs` — certificate and revocation
  sign/verify, verification from carried fields, `WAN_SIG_MAX_AGE_SECS`
- `agentmux-srv/src/backend/storage/agents.rs` — M4d-1 purge reaches
  `wan.db`; `agent_wan_keys.rs` retired after import
- `agentmux-srv/src/backend/agent_config.rs` — instance id in the env,
  publish at spawn
- `agentmux-srv/src/server/reactive.rs` — `try_cloud_relay` (carry gate); the
  HTTP path's `wan_verified = None`
- `agentmux-srv/src/muxbus/relay.rs` — relay body
- `agentmux-srv/src/muxbus/cloud_subscriber.rs` — `PendingInj`, the verifier
  before transcript-request resolution, binding to the polled agent
- `agentmux-srv/src/server/service/host_ipc.rs`, `credential.rs` — the
  host-gated approval/revocation command
- `conversation_trust_grants.rs` — tier `wan` off
- `agentmux-srv/src/backend/reactive/types.rs`, `handler.rs`, `sanitize.rs`
- CEF host + frontend: the approval/revocation window
- `CLAUDE.md` (with D2)

**agentmux-cloud**
- `muxbus/server/src/index.ts` — inject/pending fields, key and revocation
  routes
- `muxbus/server/src/store.ts` — row fields, sender account, idempotency
- new: `muxbus/server/src/wan-keys.ts` — record and revocation chain checks
- `muxbus/infrastructure/lib/constructs/muxbus-tables.ts` — key table
