# SPEC: Memory follows the agent — across upgrades, channels, accounts, and machines; plus the Armory memory refresh

**Date:** 2026-09-24
**Status:** proposed; nothing here is built. The research (§1) was
measured on this machine and in code at `agentmux` `main` @ `82d39cb83`
and `agentmux-cloud` `main` @ `fb93159`, on 2026-09-24.
**Revision history:** revised the same day after an adversarial review
(4 P1, 12 P2, 4 P3). Every finding was verified against code or on-disk
data, and all were accepted.
- **Directories:** an agent's memory directory now comes only from what
  its own spawn used. Directories shared with another agent are never
  projected, captured or deleted (§2.1.2).
- **Reconcile:** the parent is the version last projected into that
  directory, and deletion requires a matching hash (§2.1.3).
- **Storage:** a heads snapshot, blobs stored as zone files, and a
  conditional append that works across srv processes (§2.1.1).
- **Adoption:** candidates are listed by the server and confirmed by a
  human, and the MEMORY.md indexes are unioned (§2.1.4).
- **Sync:** every step that widens access goes through the integrations
  spec's consent flow; versions are signed with the WAN spec's instance
  keys; versions from new instances are quarantined (§2.2).
- **Global Memory:** follows the existing channel isolation, with an
  import step (§2.1.6).
- **Editors:** unsaved edits are protected (§2.4).
- **Also:** existing history is imported first; M0 restores the union of
  Maricon's older folders.

**Trigger:** the repo owner, on 2026-09-24, after upgrading to 0.57.2 and
reopening this agent in the new build:
> "we are doing work right now regarding conversation history, more
> robust, so it follows. we also want something similar for memory. can
> you investigate that, write spec to file, you should already see PRs in
> progress from agent3 / agentx"

and, in a follow-up:
> "there is also work (perhaps 3 weeks ago) regarding cloud-shared global
> memory .. we likely want to extend that to personal agent memory too
> ...we also want to refine the armory .. the global memory should match
> the file tile look of an agent's personal memory. tiles first, then
> expand to full. also pin the text editor to the bottom of the pane. move
> anything below to the top."

**Related:**
- `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` and the continuity
  series #3643, #3673, #3674, #3676, #3677, #3678: how the conversation
  follows the agent. This spec mirrors it for memory.
- #3693 (open, agent3): SearchHistory re-lists identity dirs on every
  discovery.
- `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md` (#3451): the
  cloud-shared Global Memory design. It is **proposed only; nothing is
  built.** agentmux-cloud #87 built its account-scoped wake, which has no
  callers yet.
- `SPEC_IDENTITY_STORE_SPLIT`, step 1b (open): moving memory and bundles
  into the global identity store.
- `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`,
  `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`,
  `SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`.
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §8: "two
  same-named agents on two machines are two agents"; linking them across
  machines must be explicit.
- The Armory memory specs: `…PERSONAL_MEMORY_FILE_TILES_2026_09_04`,
  `…CONTENT_VIEW_2026_09_11`, `…GLOBAL_MEMORY_DECLUTTER_2026_09_15`,
  `…MEMORY_TAB_MERGE_2026_08_30`.

---

## 0. Scope and summary

1. **Memory follows the agent.** An agent keeps every kind of memory it
   has across version upgrades, channel changes, provider account
   switches, renames and working-directory changes, the way its
   conversation now does.
2. **Cloud-shared Personal Memory.** Build the (so far unbuilt) cloud sync
   for Global Memory, and extend it to Personal Memory, so an agent's
   memory is the same on every AgentMux instance where the user has linked
   that agent.
3. **The Armory's Global Memory view** uses the same file tiles as an
   agent's Personal Memory: tiles first; a tile expands to the full view.
4. **Memory editors** are pinned to the bottom of the pane, and anything
   that sits below an editor today moves to the top.

**The core idea mirrors continuity: AgentMux keeps its own record of every
agent's memory.** The record lives in the global store, keyed by the
agent's UID and never by account, channel, version or working directory.
The provider's memory folder becomes a **projection** of that record:
- it is rebuilt from the record whenever the agent is spawned;
- anything the provider writes into it is captured back into the record.

An account switch or a new channel then no longer loses memory; it only
means writing the projection somewhere new.

---

## 1. Research

### 1.1 How the conversation follows the agent today

- **The record.** AgentMux mirrors every agent's stdout into the global
  transcript store, `~/.agentmux/shared/agents/transcripts/filestore.db`.
  The zones:
  - `agent:<defId>:current` holds the output and `continuity.state.jsonl`,
    a running summary kept as immutable versions (`version`,
    `created_at_ms`, `based_on`, `sha256`, `text`; #3673);
  - `agent-uid:<uid>:segments` holds `segments.jsonl`, one segment per
    spawn, recording its provider, config dir, session id, cwd, channel,
    version and rung (#3676).
- **Why a filestore log, not SQL** (durable spec §4.1.1):
  - a new table in `identity-store.db` bumps its schema version, and
    `check_schema_compat` (`migrations.rs:2333`) would lock older builds
    out of the file;
  - the global `filestore.db` takes new zones without a version bump.
- **The provider's files are a cache.** A new session that can't resume
  natively gets a continuation packet built from the record (#3643,
  #3674). A recovery candidate from a different account's dir is refused
  unless it heads the agent's segment chain (#3677). The pane says
  "Session continued · reconstructed context" (#3678).
- **The patterns to reuse:**
  - UID keys;
  - append-only JSONL in global filestore zones;
  - immutable versions with a hash and a parent;
  - tail-window reads, with corrupt lines skipped;
  - best-effort, non-blocking writes;
  - per-agent opt-out meta;
  - identity dirs listed fresh on every discovery, never cached at start
    (#3693).

### 1.2 Every memory store an agent has

| # | Store | Where | Keyed by | Scope |
|---|---|---|---|---|
| 1 | Claude auto-memory, which Personal Memory (`MemoryWrite`/`MemoryRead`) reads and writes | `$CLAUDE_CONFIG_DIR/projects/<project_dir_name(cwd)>/memory/*.md` (`native_memory_handlers.rs:63-72`). The config dir is the linked account's `…/identities/<acct>/claude`, whose `projects/` is a symlink into `shared/identities/<acct>/claude/projects` (`ensure_history_link`, `data_paths.rs:483`, re-checked at every spawn, `inject.rs:874-886`). | **account + working dir** | per account, shared across channels and versions |
| 2 | Personal Memory mirror and version history (History/Diff/Revert) | `db_agent_native_memory`, `db_agent_native_memory_versions` in `id_store` | definition id | **per channel** on every non-`stable` channel (`registry/paths.rs:63-72`) |
| 3 | Global Memory | `db_bundles` (`is_global=1`) plus `db_bundle_versions` in `id_store` | bundle id (random per instance); `name` must be unique | **per channel** off `stable`; `shared/store.db` on stable |
| 4 | Per-agent memory bundle (`memory_id`) | `db_bundles` | bundle id | per channel; m0021 mints a **new** one in each channel |
| 5 | Agent content (system prompt, soul, env) | `db_agent_content` (per version), copied to `shared/agents/definitions/<defId>.json` | definition id | global copy |
| 6 | Launch instruction files (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `.mcp.json`) | the agent's working dir `~/.agentmux/agents/<slug>-<suffix>/` | working dir | shared; **regenerated from the launching channel's Global Memory, so the last channel to launch wins** |
| 7 | Continuity summary and segments | the global filestore (§1.1) | defId / UID | global |
| 8 | Codex / Gemini / Kimi memory | not managed. Gemini's `save_memory` would write `GEMINI.md` inside the per-channel account dir, which isn't linked. | – | per channel (inferred) |

### 1.3 What each change does, measured on this machine

- **Version upgrade within a channel** (e.g. 0.57.0 → 0.57.1 on
  `stable`): everything survives, because stores 2–4 live in the
  channel's `id_store`, not the per-version `objects.db`.
- **A new local-build channel** (this agent, 0.57.0 → 0.57.2, channel
  `…a3f5ccf9` → `…f75ac48b`):
  - **Native files survived by design, not by copying.** Both channels'
    `projects/` symlinks point at one directory under
    `shared/identities/267abecd…/`; the same inode (1737505) was measured
    through both paths. That only held because Reconnect reused the same
    account id (`identity_auth_dirs.rs:234-238`). A fresh login mints a
    new id, and would have lost everything.
  - **Personal Memory history started over, and was mislabelled.** The
    new channel recorded all 5 files as `external_fs_write` ("Detected
    outside AgentMux"). Migration m0024 was meant to prevent that, but it
    runs at `bootstrap.rs:603`, before `attach_identity_stores` at `:846`,
    so account-linked dirs can't be resolved yet.
  - Global Memory went back to the system seeds, and the agent's ABF
    bundle was re-minted (91ee335e → 66e73715).
- **Account switches have already scattered memory:**
  - This agent's memory sits under four accounts. The account in use
    Sep 17–18 (`e562b87a`) holds three topics that never carried forward.
  - **Maricon's current account (`1b64d8a3`, linked 2026-09-24 10:56)
    has an empty `memory/`. Its latest memory is under `ee8af73e`, so it
    effectively has no memory right now.**
- **A cross-agent read.** m0024's backfill resolved a stale registry
  `identity_id` (b43cec34 on record 74b86143,
  `native_memory_handlers.rs:328-340, 399-417`). It saved **another
  agent's (Maricon's) files** as history rows, in three stores.
- **Rename:** survives. It changes only `instance_name`; the slug and
  working dir stay.
- **Working-directory change:** memory is lost, because the Claude
  project folder name is derived from the cwd.
- **Nothing copies memory between channels today.** The launcher's only
  copy step (`migrate_legacy_data_dir`, `data_dir.rs:139-171`) works
  within one channel.

### 1.4 Cloud-shared Global Memory: what exists

- **`SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20` (#3451) is design
  only.** Neither repo has any code for it. The design:
  - agent-writable Global Memory entries sync per account (Cognito `sub`);
  - each entry has a parent-version chain: a fast-forward is applied, a
    divergence gets a three-way merge, and a failed merge becomes a
    sibling entry "(synced conflict from X)";
  - last-writer-wins and CRDTs are explicitly rejected;
  - a `memory_updated` wake on the MuxBus WebSocket;
  - `PUT /memory/:account/entries/:id` and `GET …?since=`;
  - a DynamoDB table `muxbus-global-memory`;
  - opt-in, off by default.
- **Built groundwork:** agentmux-cloud #87 added account-scoped wakes
  (`broadcast.ts:57-90`, `ws-connect.ts:81`). There are no callers.
- **Local groundwork that already exists:**
  - both version tables have the same shape (`content_hash`,
    `parent_version_id`, `source`), and there is `line_diff`;
  - the change events `memories:changed` and `agent:memory:changed:<uuid>`;
  - the drift detector for files Claude writes itself
    (`native_memory_drift.rs`: a watcher plus a 30 s sweep);
  - the MuxBus client (`cloud_subscriber.rs`, message enum at
    `:112-128`).
- **Gaps in the sync spec, found now:**
  - entry ids are random per instance, and `name` is unique
    (`migrations.rs:1487`), so two instances that each create "Style"
    collide;
  - the Armory save path (`agent_handlers/bundle.rs:102`) has no push
    hook and no size cap;
  - DynamoDB items max out at 400 KB, while entries can be 1 MB and
    Personal files 10 MB;
  - "remove" only clears `is_global`, so sync needs delete markers.
- An older "settings sync" (yjs CRDTs,
  `PLAN_AWS_SERVERLESS_AND_WEBHOOK_DECOMMISSION:236`) is an aspiration
  only.

### 1.5 The Armory memory UI today

- **The Memory tab** (`armory-view.tsx:33-36, 144-169`) has a
  Global/Personal switch.
- **Personal Memory** (`native-memory-manager.tsx`):
  - agent cards (`MemoryAgentCard`);
  - then file tiles (`MemoryFileCard.tsx:67-115`: icon, name, badges,
    "size · age"; a grid of `auto-fill, minmax(200px, 1fr)`,
    `native-memory-manager.scss:149-253`);
  - then a detail view (`NativeMemoryHistoryPanel.tsx`), with a
    back/breadcrumb header (`:509-530`).
  - The detail view is **read-only**, with the content preview at the
    **top** and history, diff and revert **below** it (`:63-214`).
- **Global Memory** (`global-bundle-manager.tsx`) is a vertical stack of
  full-width cards:
  - a read-only `CLAUDE_CONFIG_DIR` CLAUDE.md card;
  - one card per entry, each with an always-visible 240 px preview and
    Edit/Remove/↑/↓;
  - a draft card, "+ Add Memory", and a "Combined preview" toggle.

  It edits **inline in the card** (`MemoryEditor`, `:43-93`: name,
  textarea, and Cancel/Save **below** it). It has no history UI; history
  exists only over MCP/REST (#3448).
- **The Stash drawer** (`AgentNativeMemoryModal.tsx`): its view, edit and
  history modes all put their buttons **below** the content or editor
  (`:224-287`).
- **Stale spec statuses:** FILE_TILES, AGENT_BLOCKS, FILTER_AND_SORT and
  TAB_MERGE say "Proposed" but shipped (#3000, #2917, #2929, #2844).
  CONTENT_VIEW says "not implemented" but shipped (#3218).
  AGENT_FACING_GLOBAL_MEMORY_API says history is unbuilt; #3448 built it.

---

## 2. Design

### 2.1 The memory record (Part 1: memory follows the agent)

#### 2.1.1 Storage

**The record.** A new zone per agent in the global filestore,
`agent-uid:<uid>:memory`, where `<uid>` is `db_agents.id`: the key
`SelfOwner` uses (`app_api/mod.rs:1081-1093`), and stable across channels
and versions (the per-version backfill keeps it, `agents.rs:722`). The
zone holds three files:
- **`log.jsonl`**, append-only. One line per event: `version`,
  `projected`, `captured` or `tombstone`, each with `file`, `sha256`,
  `parent`, `source`, `source_detail`, `created_at_ms`, `instance` and
  `dir_id` where it applies. **Bodies are never inline.**
- **`blob/<sha256>`**, one filestore file per distinct body. The filestore
  has zones and files but no blob API (`types.rs:12-25`), so blobs are
  files named by their hash, and writing one is idempotent.
- **`heads.json`**, a compacted snapshot: per file, the head version, its
  sha256 and whether it is deleted; per `(instance, dir_id, file)`, the
  version last projected there. It is rewritten after each append,
  under the same transaction.

Readers use `heads.json` for current state and read `log.jsonl` only for
history, so no tail window has to hold every file's head (the 64 KB
windows in continuity would miss rarely edited files).

**One writer at a time, across srv processes.** The filestore gets a
**conditional append**, run in one `write_txn`:
- it checks that `heads.json`'s head for `file` equals the caller's
  expected parent;
- it appends the event;
- it rewrites `heads.json`.

On a mismatch the caller re-reads and retries, or records a conflict
(§2.1.3). Sizes are read from the database (`line_state`), not from the
per-process cache, which goes stale across processes (`core.rs:394-402`;
the same workaround `segments.rs:24-29` uses). Captures are deduplicated
on `(file, parent, sha256)`, so N srvs capturing the same provider write
record it once.

**Sources.** Every source the current version table records is kept:
`agent`, `armory-ui`, `human`, `revert`, and `jekt` (with its TIER/TRUST
warning). New sources are `provider` (captured from disk), `adopted`,
`sync` and `legacy-build`.

**Existing history is imported first.** Before any table becomes a cache,
a one-time import copies `db_agent_native_memory_versions` (from every
channel store on the machine) into the record, deduplicated by sha256.
The known misattributed rows (§1.3) are excluded by exact row id.

**AgentMux writes are never failed by the record.** `memory_write_impl`,
the UI write RPC and revert all run the same sequence:
1. conditional-append to the record, retrying on contention;
2. if the record is unreachable after the filestore's 5 s
   `busy_timeout`, write the file anyway and log it;
3. drift capture (§2.1.3) records the file later.

The record is the source of truth for history and sync. It never blocks
a write.

#### 2.1.2 Which directory is "this agent's"

**The directory comes from the spawn, never a lookup.** A Claude memory
directory is keyed by account plus working directory
(`native_memory_handlers.rs:63-72`). It is resolved for an agent **only
from what its own spawn used**: `CLAUDE_CONFIG_DIR` in the spawn env,
plus `config.working_dir` — the values `record_segment_start` already
records (`persistent/segments.rs:52-58`).
- The directory is canonicalised: the per-channel `config_dir` maps to its
  account, then to `shared/identities/<acct>/claude/projects/<cwd-slug>/memory`.
  That canonical path is `dir_id`.
- Registry `identity_id`, the blank-working-dir fallback
  (`memory_dir_for_blank_working_dir`, `:282-317`) and name-derived
  defaults are **never** used to find an agent's memory. That code path
  caused the cross-agent read in §1.3.
- **M1 fixes the shared helper, not only m0024.** `list_all_memory_targets`
  (`:358-391`) and the drift detector stop using that path. Drift maps a
  directory to agents only through segments.

**Shared directories.** Two agents can share one memory directory: same
account plus same working directory, or agents with no linked account in
`shared/providers/claude`. On this machine, account `267abecd` holds
project folders for three agents.
- **Detecting it:** at reconcile time, if the segment log of any **other**
  UID shows the same `dir_id`, the directory is **shared**.
- **What a shared directory does not get:**
  - no projection, so the record's files aren't written into it;
  - no capture, so its files aren't attributed to either agent;
  - no deletion.
- **What it does get:** the Armory flags it as "Memory folder shared with
  <other agent>", with a fix: give the agent its own working directory.
  The agents keep working exactly as today in the meantime.

#### 2.1.3 Reconcile

**Where it runs:** in `persistent/spawn.rs`, next to `record_segment_start`
(`:488`). That is once per process spawn, where the spawn's directory is
known. Not in `inject.rs`, whose account branch is skipped for agents
without an account and runs on every turn.

**For each file**, comparing the record's head, the version last
projected into this `(instance, dir_id)` (from `heads.json`), and what is
on disk:

| Disk vs last projected | Record head vs last projected | Action |
|---|---|---|
| same | same | nothing |
| same | changed | write the head to disk; record `projected` |
| changed | same | **capture**: a new version whose parent is the **last projected** version; record `projected` |
| changed | changed | **conflict** (below) |
| absent, never projected | exists | write the head (new account, cwd, channel or host) |
| exists, never projected | none | capture (the provider wrote it) |

**The parent is always the version last projected into that directory**,
never the record's current head. A version that arrived from another
directory in the meantime therefore shows up as a conflict instead of
being overwritten.

**Conflicts:**
- The record keeps both versions. Disk keeps its file.
- The record's version is written beside it as `<stem>__conflict_<short>.md`,
  which passes `validate_filename`'s stem rule (`:449-453`), so the agent
  can read, merge and remove it with its normal memory tools.
- The **MEMORY.md index** gets one line pointing at the conflict file.
  Claude loads only the index, so an unindexed file would never be seen.
- The Armory shows the conflict with a merge view.

**Tombstones.** A deleted head removes the file from disk **only if** the
disk's sha256 equals the tombstone's parent. Otherwise it is a conflict.

**Drift capture while the agent runs.** The drift detector
(`native_memory_drift.rs`) keeps watching the directory. It captures into
the record with the same rule, using the directory's last-projected
version as the parent.

#### 2.1.4 Adopting memory from earlier accounts

**Why segments alone aren't enough.** They exist only since #3676, and
record only each agent's *current* account. So for every agent today they
miss older accounts: `e562b87a` for this agent, and `b43cec34` and
`ee8af73e` for Maricon.

**How candidates are found:**
- The **server** enumerates candidates for an agent: every
  `shared/identities/*/claude/projects/<slug of the agent's working
  dir>/memory/` directory, with file counts, dates and hashes, excluding
  directories shared with another agent (§2.1.2).
- It offers the human the list in the Armory ("Earlier memory found under
  N accounts"). **The client submits only indexes into the server's own
  list, never a path.**
- Nothing is adopted automatically, except directories the agent's own
  segments prove were its own.

**How files are merged:**
- A file that exists in several chosen directories is recorded with every
  distinct body kept as a version, newest by mtime as the head.
- The **MEMORY.md indexes are unioned, line by line**, deduplicated by
  link target, so no adopted topic becomes invisible to Claude.
  Measured: Maricon's newer index omits two topics that exist only in its
  older account.

**Who can confirm.** Confirmation goes through the host-gated window,
never a WebSocket RPC. Agents hold `AGENTMUX_AUTH_KEY`, and the
WebSocket's "human" label is assumed, not proven
(`agent_handlers/bundle.rs:95-101`). Until GHSA-6726-q276-g6f6 is fixed,
that window protects against MCP tools, not a same-user process. The
worst an agent could then do is import memory from a directory the server
listed for **that** agent.

#### 2.1.5 Unattributed callers

`SelfOwner` falls back to the slug for callers without an agent token
(`app_api/mod.rs:1086-1091`). Those writes go to the file as today, with
no record append. Drift capture then records them only if the directory
is proven for exactly one agent. Quick-launch panes without a
`db_agents` row get no record.

#### 2.1.6 Global Memory and the agent bundle

**Scoped to the same boundary as today's isolation.** Non-stable channels
are isolated on purpose, so destructive Armory testing can't touch the
real store (`paths.rs:45-72`). So:
- The Global Memory record lives in zone `global-memory:<scope>`, where
  `<scope>` is `shared` for channels that share their identity store
  (stable, or isolation off), and `channel:<ch>` for isolated channels.
- A new isolated channel starts with an **import step** in the Armory:
  "Bring Global Memory from <channel>" (a server-listed choice). Nothing
  is shared silently.
- Entries carry a stable `entry_id`. Ordering is recorded as `order`
  events holding the full id list, matching
  `ReorderGlobalBundlesCommand`.
- `db_bundles` stays the cache. A display-name clash (`name` is UNIQUE,
  `migrations.rs:1487`) is resolved in the cache only, as
  "<name> (<entry_id short>)".

**Writes from older builds.** They go to `db_bundles` without touching
the record, and there is no file on disk to detect. So each start compares
the cache with the record by `(entry_id, content hash)`, and records any
unknown row as `legacy-build`.

**The per-agent ABF bundle id** goes into a sidecar zone,
`agent-uid:<uid>:bundle`, holding `{memory_id}`. It does **not** go into
`DefinitionRecordV1`: that would need a schema bump that older builds
reject (`def_schema.rs:25-27, 70-75, 164`). m0021 reads the sidecar
before minting a new id.

#### 2.1.7 Other providers and opt-out

- **Other providers (M6):** the same record, projected into Gemini's
  memory file; Codex and Kimi when they get one.
- **Opt-out:** agent meta `agent:memoryrecord = off` keeps today's
  behaviour for that agent.

### 2.2 Cloud-shared memory (Part 2)

The unit of sync is **a record version**: a log event plus its blob.

**Who may turn it on.** Every agent holds the account's user token
(`muxbus_handlers.rs:338-340`), and a user token carries no agent or
instance identity (`auth.ts:175-180`). So every step that widens
access — turning sync on, creating a memory group, adding a UID to a
group, approving a new instance's versions — goes through the
integrations spec's **human-only consent flow** (§2.2 there): a pending
request made by the desktop, a confidential consent client, and a
relay-hosted confirm page. The relay commits nothing a user token alone
asks for.

**Who sent a version.** Versions are signed by the sending instance's
**instance key**, from the WAN spec's self-certifying instances
(`SPEC_WAN_JEKT_VERIFICATION` §2.2). The relay checks the signature on
upload, and every receiver checks it again. `instance` and `source` are
never trusted from the client. A version from an instance the receiver
hasn't approved (the WAN spec's `INSTANCE_STATUS=new`) is
**quarantined**: kept in the record but not projected, and shown in the
Armory for a human to accept.

**What is always refused:**
- system-tier Global Memory (`is_system`), on send and on receipt;
- tombstones for files the receiving instance has changed since the
  tombstone's parent (the same rule as local tombstones).

**Memory groups (Personal Memory across machines).** The identity spec
(§8) says same-named agents on two machines are two agents, so a group is
an explicit link:
- The human starts or joins a group from the Armory. The relay records
  `(account, group_id) → {uid@instance}` only after consent.
- Unlinking stops sync, and both sides keep their copies.
- Renames, working-dir changes and account switches don't affect the
  group.

**Merge rule.** From the sync spec: a fast-forward is applied; a
divergence gets a three-way line merge; a failed merge becomes a
conflict, handled as in §2.1.3, with a `__conflict_` file and an index
line.

**Cloud (agentmux-cloud):**
- **Routes:**
  - `PUT /account/v1/memory/versions` (signed events, bodies by hash);
  - `GET /account/v1/memory/versions?after=<seq>&scope=…`;
  - the consent-gated group and opt-in routes.
- **Cursor:** a **server-assigned sequence number** per `(account,
  scope)`. Receivers re-read a small overlap window, and dedupe by version
  id. Desktop clocks are never trusted for ordering.
- **Storage:** DynamoDB holds the event metadata. **Bodies go to S3** at
  `s3://…/<account>/<sha256>`, prefixed by account and only readable
  through the relay.
- **Wake** (dependencies listed in §3):
  - `broadcast.ts` gains a payload parameter; today it is hard-coded to
    `inject_available` (`:65`);
  - `ws-connect.ts:74-81` stores the account for user tokens too — the
    integrations spec's I1 change;
  - the desktop's `ServerMsg` enum (`cloud_subscriber.rs:112-128`) gains
    `MemoryUpdated`.

**Residual.** Memory is loaded as instructions (MEMORY.md into every
Claude session; Global Memory into CLAUDE.md). A compromised *approved*
instance can therefore push instructions to the account's other
machines. That is the same trust as the account itself, and it is
recorded in §4.

### 2.3 Armory: Global Memory as tiles (Part 3)

**Tiles first:**
- **The tile:** a generalized `MemoryTile` component (`{icon, title,
  badges, meta}`). `MemoryFileCard` becomes a thin wrapper over it, and
  it uses the same grid and tile CSS (`native-memory-manager.scss:149-253`).
- **Entry tiles:** one per entry, system entries first, with a "system"
  badge, "size · updated", and "from <instance>" or "quarantined" badges
  where they apply.
- **Other tiles:** the read-only `CLAUDE_CONFIG_DIR` CLAUDE.md gets a
  lock badge; "Combined preview" and "+ Add memory" are tiles too.
- **Order:** drag to reorder, recorded as an `order` event.
- The always-visible 240 px preview per card goes away.

**Expand to full.** Clicking a tile opens the full view, with the Personal
Memory back/breadcrumb header (`native-memory-manager.tsx:509-530`).
- The history panel is **split in two**:
  - `MemoryHistory` (versions, diff, revert), with a pluggable data
    source;
  - `MemoryContent` (the view or editor).

  Today `NativeMemoryHistoryPanel` contains the current content and
  builds its own model with hard-coded RPCs (`:53, 63-97`).
- New UI RPCs over `db_bundle_versions` feed Global Memory history, which
  is MCP-only today (#3448).

### 2.4 Memory editors pinned to the bottom (Part 4)

**Layout, on every memory editor surface.** A two-region flex column:
- **Top** (`flex: 0 1 auto; overflow-y: auto`): the header/breadcrumb,
  the action bar (Save, Cancel, Edit, History, Revert, Close), the name
  field, history, diff, hints and errors — everything that sits below an
  editor today.
- **Bottom** (`flex: 1 1 auto; min-height: min(240px, 50%)`): the editor
  or read-only content, filling the rest of the pane down to the bottom
  edge.

A drag handle resizes the two regions, and the split is remembered per
surface. Small panes (down to 128 px) keep working because of the
percentage floor.

**Unsaved edits are never lost.** Today the detail panel remounts on
every `agent:memory:changed` for that agent (`native-memory-manager.tsx:344, 580`).
With a dirty draft, a change event instead:
- shows a "changed since you started editing" banner, offering View
  change or Keep editing;
- keeps the draft.

A save carries its base version, and a base that has moved becomes a
conflict (§2.1.3), not an overwrite.

**Surfaces:**
- **Armory → Global Memory full view** (§2.3).
- **Armory → Personal Memory full view:** content moves to the bottom and
  history to the top. **Editing is added** (open question 3), using the
  existing `agent:memory:write_file` RPC, so M2 doesn't depend on the
  record.
- **Agent pane → Stash → Personal Memory** (`AgentNativeMemoryModal.tsx`):
  the Edit/History, Cancel/Save and Close bars
  (`:224-287, 317-323`) move to the top, and the content or textarea
  fills the bottom.
- **New Global Memory entry:** opens the full view, not an inline card.

**Keyboard:** Ctrl/Cmd+S saves, and Esc cancels (asking first when the
draft is dirty).

---

## 3. Phasing and rollout

| Phase | What ships | Depends on |
|---|---|---|
| **M0** (manual, with the human's OK) | Restore Maricon's memory: the **union** of `ee8af73e` (newest), `b43cec34` (two topics only there; **the human confirms it is Maricon's**, since it is the stale id from the m0024 bug) and `d5ba7d63` (one differing file, kept as a `__conflict_` sibling), with a unioned MEMORY.md, written into `1b64d8a3`'s shared `projects/…/memory`. The current dir is empty, so nothing is overwritten; a backup is taken first. | — |
| **M1** (fixes) | stop using registry `identity_id`, the blank-working-dir fallback and name-derived dirs in `list_all_memory_targets` and drift; **a new post-attach step** after `attach_identity_stores` replaces m0024's backfill (m0024 has already run in existing channels, so moving it wouldn't help); first sighting recorded as `adopted`; the misattributed rows deleted by exact id | — |
| **M2** (UI) | Global Memory tiles and full view; history and content split; editors pinned to the bottom; dirty-draft protection; Personal Memory editing through the existing RPC; Global Memory history RPCs | — |
| **M3** (record) | the filestore conditional append and database-read sizes; the `agent-uid:<uid>:memory` record (log, blobs, heads); history import; reconcile at spawn; shared-dir detection; drift into the record; server-listed, host-confirmed adoption with index union; opt-out | M1 |
| **M4** (Global Memory record) | `global-memory:<scope>` with entry ids, order events and the import step; legacy-build capture; the bundle sidecar | M3 |
| **M5** (cloud sync) | the integrations spec's I1 `ws-connect` change and consent flow; the WAN spec's instance keys; relay routes, sequence cursor, S3 bodies, broadcast payload, `MemoryUpdated`; Global Memory first, then memory groups | M3, M4; integrations I1; WAN instance keys |
| **M6** (other providers) | project the record into Gemini's memory file | M3 |

**Compatibility:**
- Builds older than M3 never read the record zones. Their file writes are
  captured by the next M3 spawn. Their `db_bundles` writes are captured
  as `legacy-build` (§2.1.6).
- Concurrent builds of one channel use the conditional append and
  database-read sizes (§2.1.1), so they never fork a head.
- Everything that relies on the host-gated window (adoption confirmation,
  and approving sync instances) carries the GHSA-6726-q276-g6f6 caveat.

---

## 4. Residuals and open questions

**Residuals:**
- **Shared memory directories** don't get the record's benefits until the
  agents are given separate working directories (§2.1.2).
- **Same-user processes.** Until GHSA-6726-q276-g6f6 is fixed, a
  same-user process can confirm adoption or approve instances through
  the host channel. Adoption is limited to directories the server lists
  for that agent.
- **Approved instances are trusted like the account.** A compromised
  approved instance can push memory, which is loaded as instructions, to
  the account's other machines (§2.2).
- **Unattributed writers** (no agent token) aren't recorded unless their
  directory is proven for exactly one agent (§2.1.5).

**Open questions:**
1. **Adoption:** is the server-listed, human-confirmed list (§2.1.4)
   enough, or should some directories be adopted automatically?
2. **Conflict files on disk** (`<stem>__conflict_<short>.md` plus an
   index line) vs conflicts kept only in the Armory. Recommended: on disk,
   so the agent sees them.
3. **Editing Personal Memory in the Armory** reverses `…CONTENT_VIEW` §6.
   Confirm.
4. **Sync scope:** Global Memory and memory groups only, or also ABF
   bundle instructions?
5. **Memory groups across accounts** (sharing an agent's memory with
   another person): out of scope for v1?
6. **Retention:** keep every version, or the current "at least 50
   versions / 90 days" rule? Blob garbage collection follows from that
   choice. This must be decided before M3.
7. **Where Global Memory lives in the Armory:** stay under Memory
   (recommended), or move to the Bundles tab as the naming-consolidation
   spec planned?

## 5. Tests (summary)

**Record:**
- The conditional append refuses a stale parent.
- Three srv processes capturing the same write produce one version.
- A process sees another process's appends (database-read size).
- `heads.json` stays consistent with the log after a crash between
  appends.
- History import is deduplicated by hash; the misattributed rows are not
  imported.
- The record being unavailable never fails a MemoryWrite.

**Directories:**
- Only spawn-env directories are used; registry `identity_id` and the
  blank-working-dir fallback are never used.
- A directory shared with another UID is neither projected nor captured.
- A new account dir is filled from the record.
- A working-dir change projects into the new folder.

**Reconcile:**
- Every row of the §2.1.3 table.
- The parent is the last-projected version, so a concurrent remote
  version produces a conflict, never a silent overwrite.
- A tombstone deletes the file only when the disk hash matches its
  parent.
- Conflict file names pass `validate_filename`.
- The index gets the conflict line.

**Adoption:**
- Candidates are enumerated by the server; the client submits indexes
  only.
- MEMORY.md indexes are unioned.
- Directories shared with another agent are excluded.
- A WebSocket RPC can't confirm adoption.

**Global Memory record:**
- The scope follows isolation.
- The import step works.
- Order events are recorded.
- Legacy-build rows are captured.
- The bundle sidecar prevents re-minting.

**Sync:**
- A user token alone can't enable sync or join a group.
- An unsigned version is rejected; so is a version signed by another
  instance's key.
- A new instance's versions are quarantined.
- `is_system` is refused.
- Stale tombstones are refused.
- The cursor survives clock skew.
- Bodies go through S3 under the account prefix.
- The wake reaches desktops.

**UI:**
- Tiles render, and expand to the full view.
- The top and bottom regions resize; the floor holds at 128 px.
- A dirty draft survives a change event.
- A save with a moved base becomes a conflict.
- Ctrl/Cmd+S saves.
- The Stash drawer's Close button moves to the top.
- Personal Memory editing uses the existing RPC.
