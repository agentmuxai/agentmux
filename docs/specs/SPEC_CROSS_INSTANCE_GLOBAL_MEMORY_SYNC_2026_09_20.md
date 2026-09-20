# Spec: Cross-Instance Global Memory Sync

**Status:** proposed — research/design only, nothing implemented.
**Date:** 2026-09-20
**Verified against:** `agentmux` at `e25bbedc6` (origin/main, fetched today),
`agentmux-cloud` at `ec77c7c6` (sibling checkout, 2026-09-18 — a couple days
stale but architecture-relevant paths don't churn that fast; re-verify exact
line numbers before implementation).
**Related:** `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md` (the
agent-facing write path this spec extends across machines — its own Phase 3
gating question and its unbuilt `GlobalMemoryHistory`/`Diff`/`Revert` read
side are both load-bearing prerequisites here, see §4), `SPEC_GLOBAL_MEMORY_
SYSTEM_TIER_2026_08_24.md` (the isolation this spec must not weaken),
`SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` (prior
self-audit of what MuxBus is and isn't — this spec's transport section
depends on its findings), `SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md` (the
per-channel data-zone model this spec's scope decision (§2.1) has to sit on
top of), `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` (the
per-provider delivery step downstream of sync that this spec does not
change), `agentmux-cloud/docs/REPORT_AGENTMUX_CLOUD_MONETIZATION_2026_05_30.
md` (the only prior mention of "settings sync" anywhere in either repo — a
one-line aspiration, not a design, see §1).

## 0. The ask

A user runs multiple separate AgentMux instances (different machines, or
different local "channels" on the same machine — today's isolation unit per
`SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md`). If an agent on instance A
writes a Global Memory entry via `GlobalMemoryWrite`, it should eventually
show up as a Global Memory entry on instances B–E too, without a human
manually copying it over.

## 1. Current state — confirmed, not assumed

**Storage is local and per-instance, full stop.** A Global Memory entry is a
row in `db_bundles` (`agentmux-srv/src/backend/storage/bundles.rs:27-90`)
with `is_global=true`, living in a SQLite DB scoped to one local
`~/.agentmux/channels/<channel>/` install. There is no workspace/org UUID
that spans machines — "workspace" in this codebase means the local UI
session container (`server/service/workspace_lifecycle.rs`), not a
multi-tenant identity. Versioning already exists, but locally only:
`bundle_upsert_with_version` / `bundle_versions.rs` track `content_hash`
(SHA256) and `parent_version_id` in an append-only chain — this is the shape
any cross-instance version model should mirror rather than reinvent.

**Delivery to a running agent** goes `db_bundles` →
`format_global_bundle_block` (`bundles.rs:152`) →
`build_config_files()` (`agent_config.rs:60-131`) → a provider-specific
startup file (`CLAUDE.md`, `AGENTS.md`, etc., per
`backend/providers.rs`) — written once, at agent launch. Kimi has
`startup_instructions_filename: None` (`providers.rs:472`) and gets no file
at all. Sync does not need to touch this step; it only needs to land rows in
`db_bundles` before the next agent launch.

**MuxBus is the only existing outbound cloud channel, and it is a
message bus, not a data-sync channel.** Locally,
`agentmux-srv/src/muxbus/cloud_subscriber.rs:4-23` holds a persistent,
authenticated (Cognito OAuth/PKCE, OS keychain) WebSocket to
`wss://muxbus-ws.agentmux.ai` with reconnect/backoff already built. On the
cloud side, `agentmux-cloud/muxbus/server/src/broadcast.ts:39-91` sends a
single zero-metadata `{type:"inject_available"}` wake event to **every open
connection globally** — the backing DynamoDB connections table has no
ownership column (`muxbus/infrastructure/lib/constructs/muxbus-tables.ts:
77-91`). Actual content delivery is a separate authenticated REST poll,
`GET /reactive/pending/:agent_id`. `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_
REMOTE_INVOCATION_2026_07_29.md` already self-audited this: cross-instance
enumeration and RPC are explicitly non-goals of MuxBus as it exists today.

**The cross-instance identity that already exists is `account_user_id`**
(the Cognito `sub`). `agentmux-cloud/muxbus/server/src/agent-ownership.rs
[ownership.ts]:1-19` models one account owning many `agent_id`s, and
`auth.ts:44-59` resolves it per request. This is the natural sync scope key
— there is no separate org/team concept yet (README.md:21 lists
Team/Enterprise as future).

**No document/blob store with versioning exists in agentmux-cloud today.**
The DynamoDB tables (`muxbus-messages`, `-agents`, `-injections`, `-quota`,
`-connections`, `-processed-github-events`, `-login-relay`) are all
message/metering tables, not a config-doc store.

**The only prior mention of this feature anywhere is aspirational.**
`agentmux-cloud/docs/REPORT_AGENTMUX_CLOUD_MONETIZATION_2026_05_30.md:187`
floats "yjs CRDT over WebSocket... settings sync is a trivial sub-case" as
one bullet in a monetization pitch. There is no `yjs` dependency anywhere in
either repo, no design detail beyond that sentence, and it's bundled
alongside a distinct "WAN pairing relay" feature (direct live-instance-to-
live-instance relay, see `PLAN_AWS_SERVERLESS_AND_WEBHOOK_DECOMMISSION_2026_
05_30.md:144,175,234-235`) that this spec deliberately does **not** depend
on — WAN pairing requires two devices online simultaneously; MuxBus's
always-on cloud relay does not.

## 2. Design

### 2.1 Scope

Sync applies to **agent-writable Global Memory only** (`is_global=true,
is_system=false` rows — the tier `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_
09_15.md` added an MCP API for). System-tier entries (`is_system=true`) are
AgentMux-authored and already reach every install through the normal
release/update channel; syncing them here would be solving a distribution
problem with a sync mechanism, and risks a lower-privileged sync path
accidentally becoming a write vector into a tier that
`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` deliberately isolated.
Out of scope, not "phase 2" — revisit only if a real need shows up.

Sync scope key is `account_user_id` (§1) — every channel/instance
authenticated to the same MuxBus account is one sync group. This is coarser
than "one physical workspace" (a user's "throwaway test channel" syncs too),
which is flagged as an open question in §5, not resolved here.

### 2.2 Conflict & version model — entry-level, not whole-doc

Each Global Memory entry (one `db_bundles` row, keyed by its existing id/
name) syncs independently. This — not CRDT machinery — is what actually
avoids most conflicts in practice: two instances writing about different
topics touch different entries and never collide. Recommend against
building the yjs-CRDT pipeline from the monetization report as a first cut;
it solves concurrent keystroke-level co-editing of a single document, which
isn't this feature's shape (agent writes are atomic, not live-typed), and it
would be new infrastructure with no existing precedent in either repo.

Per entry, extend the existing local version chain
(`content_hash`/`parent_version_id`, `bundle_versions.rs`) with a
cloud-assigned version cursor. On sync:

- **Fast-forward**: incoming version's `parent_version_id` matches the
  local head → apply directly, `Store::bundle_upsert` + version-chain
  append (existing machinery, unchanged).
- **Divergence** (both sides moved past the same parent): do **not**
  silently overwrite either side — the exact failure mode the web research
  for this spec flagged as the most common real-world sync bug (naive
  last-write-wins silently discarding concurrent edits; see Sources). Two
  options, in order of preference:
  1. Attempt a textual three-way merge (diff3 against the common parent,
     the same technique Obsidian uses for Markdown files) when the two
     versions touch disjoint regions of the entry's text.
  2. If the merge is ambiguous, write the incoming version as a sibling
     entry (e.g. `<name> (synced conflict from <instance>)`) instead of
     dropping it, and leave both visible — mirrors Obsidian's
     conflict-file fallback. A human or agent resolves it manually later.

This requires `GlobalMemoryHistory`/`Diff`/`Revert` (or an equivalent
Armory UI surface) to actually exist so a conflict is visible — per
`SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md`'s own Phase 3 note, the
read side of the audit trail (`bundle_version_list`/`bundle_version_get`)
exists and is tested but isn't exposed via any MCP tool or UI today. That
gap becomes a hard prerequisite here, not just a nice-to-have — see §4.

### 2.3 Transport — extend MuxBus, don't build a new channel

Reuse the already-shipped, already-authenticated, already-reconnecting
MuxBus WebSocket (`cloud_subscriber.rs`) rather than the separate WAN-
pairing plan (§1). Add one new wake message type, `memory_updated`,
parallel to the existing `inject_available`.

**Blocking prerequisite, not optional polish**: `broadcast.ts` must stop
fanning out globally and instead scope delivery to `account_user_id`
(`muxbus-tables.ts`'s connections table needs an ownership column it
currently lacks, §1). Shipping account-scoped memory sync on top of a
literally-global wake broadcast would leak "an account somewhere just wrote
memory" metadata to every connected instance on the service, and more
importantly means the fix can't be deferred past this feature — get it right
here since it's needed for correctness anyway, not just as a hardening
pass.

New cloud REST endpoints, mirroring the existing `/reactive/pending/
:agent_id` poll pattern:

- `PUT /memory/:account_id/entries/:entry_id` — push one version (content,
  content_hash, parent_version_id, source instance id).
- `GET /memory/:account_id/entries?since=<cursor>` — pull all versions past
  a cursor, for both the wake-triggered pull and cold-start/reconnect
  catch-up.

### 2.4 Cloud storage

New DynamoDB table (e.g. `muxbus-global-memory`), `PAY_PER_REQUEST` with
PITR enabled — matching every existing table's convention in `muxbus-
tables.ts`. PK `account_user_id`, SK `entry_id`, storing latest content plus
the version chain (either inline or a companion `-versions` table using the
same append-only shape as `muxbus-messages`).

### 2.5 Local flow

`GlobalMemoryWrite` (existing REST route, `app_api/mod.rs`) is unchanged at
the write step. Add: after the local `bundle_upsert_with_version` commit, a
background task pushes the new version to `PUT /memory/.../entries/...`
using the same Cognito token already held for MuxBus — no new credential
type. On success, record the cloud-assigned cursor (new column, or a new
`db_bundles_sync_state` table — don't overload `bundle_versions.rs`'s
existing shape, which has no concept of "cloud cursor" today).

On receiving a `memory_updated` wake: pull deltas since the last local
cursor, apply per §2.2, tag provenance as "synced from MuxBus" plus
originating instance, so `bundle_versions.rs`'s existing provenance field
(already used for agent-slug attribution per `SPEC_AGENT_FACING_GLOBAL_
MEMORY_API_2026_09_15.md`) also distinguishes local writes from
sync-applied ones.

**No hot-reload of already-open agent sessions.** Startup files are only
(re)written at agent launch today (§1) — this spec doesn't change that. A
sync landing mid-session takes effect the next time an agent on that
instance is opened, same as any other Global Memory edit.

**Offline instances catch up on reconnect** via the cursor-based
`GET .../entries?since=` pull, using `cloud_subscriber.rs`'s existing
reconnect/backoff logic as the trigger point.

### 2.6 Security prerequisites

- The `broadcast.ts` account-scoping fix (§2.3) is required, not deferred.
- `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`
  flagged `ENFORCE_AGENT_BINDING` as unset — cloud-side authorization that
  an instance can only push/pull its own `account_user_id`'s entries needs
  to be real (not just "the token happens to carry the right claim and
  nothing double-checks it") before memory content — which may include
  anything an agent chose to write, potentially sensitive — is stored
  cloud-side at all.

### 2.7 UX / opt-in

Default **off**. Unlike MuxBus's existing text-injection relay, this ships
Global Memory content off the local machine to `agentmux-cloud` — an
explicit per-instance toggle (Armory settings) is warranted, plus a manual
"sync now" affordance for users who don't want always-on background sync.
Note the monetization report frames "settings sync" as a paid-tier feature
(§1) — whether this ships free or gated is a product decision this spec
doesn't make.

## 3. Non-goals

- Real-time collaborative co-editing of a single entry (this is atomic
  agent writes, not live multi-cursor typing — no CRDT needed for that
  reason, see §2.2).
- Team/org-wide sharing across different Cognito accounts — no org concept
  exists yet (§1); scope is one account's own instances only.
- Syncing system-tier Global Memory (§2.1).
- The WAN-pairing P2P relay described in `PLAN_AWS_SERVERLESS_AND_WEBHOOK_
  DECOMMISSION_2026_05_30.md` — a separate, already-planned feature this
  spec doesn't depend on or block.
- Hot-reloading a running agent's already-written startup file mid-session.
- Building the yjs-CRDT pipeline from the monetization report as a first
  cut (§2.2) — revisit only if entry-level merge + conflict-copy proves
  insufficient in real use.

## 4. Open questions — not decided here

1. **Sync granularity**: per-account (every channel on every machine under
   one login) vs. some narrower per-declared-workspace grouping. This spec
   defaults to per-account because that's the only identity that already
   spans machines (§1) — but that means a user's intentionally-isolated
   "test channel" syncs too, which may surprise them. No workspace-UUID
   concept exists to do better without adding one.
2. **This spec structurally depends on `SPEC_AGENT_FACING_GLOBAL_MEMORY_
   API_2026_09_15.md`'s own open Phase 3 question** (agent write-access
   gating) and its unbuilt history/diff/revert read side — a sync feature
   that can silently create conflict-copy entries needs those visible
   somewhere. Implementation order matters: that spec's Phase 0 (audit
   trail) and read-side exposure should land before or alongside this one,
   not after.
3. **Billing/tier gating** (§2.7) — product decision, not engineering.
4. **Merge algorithm choice** (§2.2) — diff3 three-way merge is proposed by
   analogy to Obsidian's Markdown handling, not yet validated against this
   codebase's actual entry content shapes (which may be closer to
   structured instructions than prose).

## 5. Test plan (for implementation time)

- Unit: version-vector merge logic — same-parent fast-forward, diverged-
  parent conflict-copy creation, and a never-silently-drop-content
  invariant test (the regression test for the exact failure mode §2.2
  argues against).
- Integration: two local instances (two channels) authenticated to the same
  test MuxBus account; write on A, assert B receives the scoped
  `memory_updated` wake, pulls, applies via `Store::bundle_upsert`, and
  that a newly-launched agent on B has the entry in its startup file.
- Security (regression for §2.6): an instance authenticated as account X
  cannot push or pull account Y's entries; a `memory_updated` wake for
  account X is never delivered to a connection authenticated as account Y
  (this is the account-scoping fix to `broadcast.ts` itself, testable
  independently of the rest of this feature).
- Offline/reconnect: instance offline during a remote write; on reconnect,
  cursor-based pull catches it up fully with no manual intervention.

## 6. LAN transport (no-MuxBus path) — design sketch, not sequenced ahead of §2

For users who don't want Global Memory content touching `agentmux-cloud` at
all, Tier 3 (`agentmux-srv/src/backend/lan_discovery.rs` — mDNS/DNS-SD
`_agentmux._tcp.local.` + UDP broadcast fallback on port 47891, opt-in via
`network:lan_discovery`, default off) is the only existing alternative to
MuxBus for reaching another instance. It is **peer-to-peer discovery, not a
data-sync channel** — the same distinction as MuxBus being a message bus
rather than an RPC bus (`SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_
INVOCATION_2026_07_29.md`, Tier 3 audit row). Reusing it for Global Memory
sync means building genuinely different infrastructure from §2–§2.7, not
just swapping the transport under the same protocol.

### 6.1 What's different from the cloud path

- **No durable relay.** The cloud path gets store-and-forward for free from
  DynamoDB (§2.4) — an offline instance catches up via a cursor pull on
  reconnect (§2.5). LAN has no server: if instance A writes while B is
  offline, nothing holds that delta until both are on the same LAN and
  online simultaneously. Either (a) accept that LAN sync only converges
  when peers are concurrently reachable, or (b) have one peer
  opportunistically hold and forward deltas for instances it has
  previously seen. (b) is a real design commitment — some instance becomes
  a transient store-and-forward relay for its LAN peers — not a small
  addition, and is explicitly not proposed as part of a first cut (§6.3).
- **No `account_user_id`-equivalent scope key.** §2.1's sync group ("every
  channel under one Cognito login") doesn't exist on LAN — mDNS/UDP finds
  *candidates* on a subnet, not "my other instances." LAN sync needs its
  own pairing step to establish which discovered peers are actually
  trusted to exchange memory content, not just which ones happen to
  respond to a broadcast.
- **The trust model needs the fix `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_
  REMOTE_INVOCATION_2026_07_29.md` §2 already proposed, as a hard
  prerequisite here.** Today's `auth_key` is broadcast in plaintext in the
  mDNS TXT record (`lan_discovery.rs`) — an accepted tradeoff for that
  spec's current text-injection use case, not an acceptable one for
  silently pulling and merging another instance's memory content. That
  spec's proposed replacement — a stable per-instance identity keypair,
  mDNS/UDP advertising only a public identity, with the actual auth
  exchange happening post-discovery and pinned to that identity
  (Syncthing's device-ID-pinning shape) — must land before LAN memory
  sync, not alongside it.

### 6.2 What's reusable from §2

The entry-level version/merge model (§2.2 — per-entry `content_hash`/
`parent_version_id`, fast-forward on matching parent, three-way merge or
conflict-copy sibling on divergence) is transport-agnostic: it only needs a
way to ask a peer "give me your entries since version X" and a way to push
"here's my new version of entry Y." Both are just different request
handlers than the cloud REST endpoints in §2.3 — same merge logic, invoked
from a LAN HTTP handler on the existing Tier-3 connection instead of from
`cloud_subscriber.rs`'s wake handler.

### 6.3 Proposed shape (sketch — needs its own spec before implementation)

- **Pairing, not discovery, is the trust boundary** — mirrors §2's
  "discovery finds candidates, a separate trust step decides who to talk
  to" principle, using the same Syncthing precedent already cited in the
  discovery spec. A user explicitly pairs two instances once (e.g.
  confirming a fingerprint shown on both sides); paired instances form the
  LAN sync group, independent of and narrower than "everything answering
  on the subnet."
- **Push-based, best-effort sync on connect**, not a persistent
  eventual-consistency guarantee: when a paired peer is discovered
  reachable, exchange version cursors and sync deltas immediately in both
  directions, then rely on mDNS re-announcement to catch the next
  reconnect. No background relay role by default — that's option (b) from
  §6.1 and should be a deliberately separate follow-up, not bundled into a
  first LAN-sync cut.
- **Same conflict-copy fallback as §2.2** — nothing about LAN transport
  changes what happens when two peers diverge from a common parent
  version; only how the delta gets from one instance to the other.

### 6.4 Sequencing

Gated on the LAN pinned-identity trust upgrade (§2 of `SPEC_MUXBUS_MULTI_
TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`) landing first — treat
that as this section's own hard prerequisite, the same way §2.6 treats the
MuxBus broadcast-scoping fix as blocking for the cloud path. Recommend
building the cloud/MuxBus path (§1–§5) first: it's simpler, already has
durable persistence, and validates the entry-level merge protocol before a
second, harder-to-get-right transport is added on top of it. LAN sync is
additive once that protocol exists — the merge logic doesn't need to be
redesigned per transport, only the discovery/trust/delivery layer
underneath it.

## Sources consulted (external best-practice research)

- [OT vs CRDT in 2026: Multiplayer Algorithm Guide](https://www.taskade.com/blog/ot-vs-crdt)
- [CRDTs vs Operational Transformation: A Practical Guide (HackerNoon)](https://hackernoon.com/crdts-vs-operational-transformation-a-practical-guide-to-real-time-collaboration)
- [Deciding between CRDTs and OT for data synchronization](https://thom.ee/blog/crdt-vs-operational-transformation/)
- [VS Code Settings Sync docs](https://code.visualstudio.com/docs/configure/settings-sync)
- [Obsidian Sync — Conflict Resolution (DeepWiki)](https://deepwiki.com/vrtmrz/obsidian-livesync/4.2-conflict-resolution)
- [Troubleshoot Obsidian Sync](https://obsidian.md/help/sync/troubleshoot)
