# SPEC: Memory follows the agent — across upgrades, channels, accounts, and machines; plus the Armory memory refresh

**Date:** 2026-09-24
**Status:** proposed; nothing here is built. The research (§1) was
measured on this machine and in code at `agentmux` `main` @ `82d39cb83`
and `agentmux-cloud` `main` @ `fb93159`, on 2026-09-24. An adversarial
review is pending.
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

**Where it lives.** A new zone per agent in the global filestore:
`agent-uid:<uid>:memory`, where `<uid>` is `db_agents.id`, the key
`SelfOwner` already uses (`app_api/mod.rs:1081-1093`). It holds
`memory.jsonl`, an append-only log of immutable file versions:

```json
{"file":"feedback_x.md","version":"v_…","parent":"v_…"|null,"sha256":"…","deleted":false,
 "content":"…","source":"agent|armory-ui|provider|adopted|sync","source_detail":"…",
 "created_at_ms":…,"instance":"<install id>"}
```

- **The newest version per file** (by parent chain, then `created_at_ms`)
  is the current content.
- **Deletes are tombstones** (`deleted: true`).
- **Size cap:** 10 MB per file, matching the current cap. Bodies over
  256 KB are stored as filestore blobs and referenced by hash, so the log
  stays readable in a tail window.
- **No SQL schema change** (the continuity lesson, §1.1). Older builds
  never touch the zone.
- **The per-channel tables become caches** that the Armory reads:
  `db_agent_native_memory` and `…_versions` are rebuilt from the record,
  and are no longer the source of truth.
- **Reads use the UID only.** An agent's memory is resolved from its UID,
  never from a registry `identity_id`. That removes the stale-link
  cross-agent read (§1.3).

**Projection to the provider's folder.** At every spawn, after the
account is resolved (`inject.rs`, next to `link_history_if_isolated`),
`reconcile_memory(uid, resolved_dir)` runs:
1. Read the record's current files and the resolved dir's files (account
   + cwd).
2. **A file on disk whose hash is not in the record** was written by the
   provider (or by hand). Capture it into the record as a new version
   with `source: provider` and its parent set to the record's head. The
   drift detector (§1.4) keeps doing this while the agent runs, but now
   writes to the record.
3. **A record head that isn't on disk** — new account, new cwd, new
   channel, or edited elsewhere — gets written to disk.
4. **Both changed since the last reconcile.** Keep both: disk wins the
   filename, and the record's version is written as
   `<name>.agentmux-conflict-<short>.md` and logged. Never merge silently.
5. **A tombstoned file** is removed from disk.

The last reconciled head per file is kept in the record (`reconciled`
events), so step 4 can tell "changed here" from "changed there".

**Writes.** `memory_write_impl`, the UI write RPC, and revert all append
to the record first, then write the file, then update the cache. The
record is the commit point.

**Adopting memory scattered across old accounts.** Once per agent, when
the record is empty, the record adopts files from **provably this
agent's** previous memory dirs only:
- the config dirs in its segment chain (`agent-uid:<uid>:segments`,
  #3676), plus its working dir;
- never a registry `identity_id`;
- for a file found in several dirs, the newest mtime wins, and the
  others are kept as versions (`source: adopted`, `source_detail` = the
  dir);
- the Armory shows one notice: "Imported memory from N earlier accounts".

**Agents that predate segments** (and so Maricon's case today) get a
human-confirmed adoption instead: the Armory lists candidate dirs for the
agent's working dir under every account in `shared/identities/*`, with
file counts and dates, and the human picks which to import. No automatic
guessing, because the m0024 backfill showed that guessing crosses agents.

**Fixes that ship first** (§3, M1):
- m0024 runs after `attach_identity_stores`, not before;
- the backfill stops using registry `identity_id`;
- the first sighting of a file in a new channel is recorded as `adopted`,
  not `external_fs_write`;
- the wrongly attributed Maricon rows in another agent's history are
  deleted, by exact row id.

**Global Memory follows too.** Stores 3 and 4 are per channel only
because `SPEC_IDENTITY_STORE_SPLIT` step 1b is unfinished. Rather than
bumping `identity-store.db`'s schema, Global Memory also gets a record in
the global filestore: zone `account:global-memory`, one log of entry
versions keyed by a **stable entry id** (§2.2). Each channel's
`db_bundles` becomes a cache of it. Launch instruction files are then
composed from the same content in every channel, so "the last channel to
launch wins" stops mattering. The per-agent ABF bundle (store 4) stores
its `memory_id` in the global definition record, so channels stop
re-minting it (the "DefinitionRecordV1 gap").

**Other providers.** The same record, projected into each provider's own
memory location where one exists:
- Gemini: `GEMINI.md` inside the account dir;
- Codex and Kimi: none today.

This is phase M6, after Claude.

**Opt-out.** Agent meta `agent:memoryrecord = off` keeps today's
behaviour for that agent.

### 2.2 Cloud-shared memory (Part 2)

Build `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC` on the record from §2.1,
then extend it to Personal Memory. The unit of sync is **a record
version**, not a table row.

**What syncs:** versions, with their parent chains, of:
- Global Memory entries: zone `account:global-memory`;
- Personal Memory files, for agents the user has **linked across
  machines**.

**Linking an agent across machines (identity spec §8).** Two same-named
agents on two machines are two agents. So Personal Memory syncs only
between UIDs the user has explicitly joined into a **memory group**:
- In the Armory, Personal → agent → "Share memory with…", the user picks
  the same agent on another of their instances (listed from the cloud),
  or starts a new group.
- The group has a random `memory_group_id`. Each member UID's record
  stores it.
- A rename, a working-dir change or an account switch doesn't affect the
  group.
- Unlinking stops sync; both sides keep what they have.

**Keys and conflicts:**
- **Global Memory** entries get a stable `entry_id` (a UUID minted once,
  carried in every version); `name` is a display field. Two instances
  creating "Style" independently produce two entries. On receipt, a name
  clash is shown as "Style (from <instance>)" until the human renames or
  merges — no silent overwrite.
- **Personal Memory** is keyed by `(memory_group_id, filename)`.
- **Merge rule, both kinds** (the sync spec's §2.2): a fast-forward is
  applied; a divergence is three-way merged by line; a failed merge
  becomes a conflict sibling (`<name>.agentmux-conflict-<instance>.md`,
  or a sibling entry for Global Memory).
- **Tombstones** sync like any other version.

**Cloud (agentmux-cloud):**
- **Routes:**
  - `PUT /account/v1/memory/versions` (a batch of versions);
  - `GET /account/v1/memory/versions?since=<cursor>&scope=global|group:<id>`;
  - `POST /account/v1/memory/groups`, and the join, leave and list routes
    for groups.
- **Auth:** the account's user token (`accountUserId ?? userId`). Group
  membership is checked per request.
- **Storage:** DynamoDB for version metadata, PK `account#scope`, SK
  `created_at#version`. **Bodies go in S3**, keyed by `sha256`, because
  DynamoDB items max out at 400 KB (§1.4).
- **Wake:** `memory_updated` through #87's account-scoped broadcast. The
  desktop pulls past its cursor on a wake and on reconnect.

**Desktop:**
- **Push:** a background step after each record append (every write path
  plus drift capture) sends the new versions.
- **Pull:** new versions are applied to the record, then reconciled into
  the running agent's folder (§2.1). Running agents see Global Memory
  changes at their next launch, as now.
- **Opt-in:** off by default, per instance, with "Sync now".
- **What never syncs:** system-tier Global Memory (`is_system`), which
  every build seeds for itself.

**The trust question stays with the WAN work.** Sync is between the
account's own instances, authenticated by the account's token. Any agent
holding `MUXBUS_TOKEN` could push versions into the account's memory —
the same residual as today's flat relay exposure. So:
- versions received from sync carry `source: sync` and the sending
  instance;
- the Armory shows "Changed on <instance>" on synced versions;
- nothing received from sync is executed or treated as instructions
  beyond what the same text would get locally.

### 2.3 Armory: Global Memory as tiles (Part 3)

**Tiles first.** Global Memory gets the Personal Memory file-tile grid:
- the same CSS grid and tile styles (`native-memory-manager.scss:149-253`);
- a generalized `MemoryTile` component, taking `{icon, title, badges,
  meta}`, which `MemoryFileCard` becomes a thin wrapper over;
- one tile per entry, system entries first, with a "system" badge,
  "size · updated" meta, and a "synced from <instance>" badge when that
  applies;
- the read-only `CLAUDE_CONFIG_DIR` CLAUDE.md becomes a tile with a lock
  badge;
- "Combined preview" becomes a tile;
- "+ Add memory" is the last tile;
- order (↑/↓) becomes drag-to-reorder on the tiles, keeping the existing
  move RPC.

**Expand to full.** Clicking a tile opens the full view, with the same
back/breadcrumb header as Personal Memory ("← All global memory"):
- **the top area scrolls:**
  - actions (Save, Cancel, Remove, Rename);
  - version history, diff and revert — the Personal Memory history panel
    with a pluggable data source, plus new UI RPCs over
    `db_bundle_versions` (history is MCP-only today, #3448);
- **the editor is pinned to the bottom** (§2.4).

**Declutter.** The always-visible 240 px preview per card goes away. The
tile shows the name and meta; the full view shows the content.

### 2.4 Memory editors pinned to the bottom (Part 4)

**The rule, for every memory editor surface.** The pane is a two-region
flex column:
- **Top region** (`flex: 0 1 auto; overflow-y: auto`): the
  header/breadcrumb, the action bar (Save/Cancel/Edit/History/Revert),
  the name field, version history, diff, hints and errors — everything
  that sits below an editor today.
- **Bottom region** (`flex: 1 1 auto; min-height: 240px`): the editor
  (or read-only content) alone, filling the rest of the pane down to the
  bottom edge. A drag handle between the regions resizes them, and the
  split is remembered per surface.

**Where it applies:**
- **Armory → Global Memory full view** (§2.3): the editor at the bottom.
- **Armory → Personal Memory full view.** Today it is read-only, with
  content on top and history below. Content moves to the bottom and
  history to the top. **This spec also adds editing** to this view,
  matching Global Memory: the same editor, writing through the record
  (§2.1). That reverses `…CONTENT_VIEW` §6's "no editing in the Armory"
  (open question 3).
- **Agent pane → Stash → Personal Memory**
  (`AgentNativeMemoryModal.tsx`): the Edit/History and Cancel/Save bars
  move above the content; the `<pre>`/`<textarea>` fills the bottom.
- **New Global Memory entry:** the draft opens the same full view, not an
  inline card.

**Keyboard:** Ctrl/Cmd+S saves and Esc cancels, since the buttons are no
longer next to the cursor.

---

## 3. Phasing and rollout

| Phase | What ships | Depends on |
|---|---|---|
| **M0** (now, manual) | Restore Maricon's memory from `ee8af73e` into its current account's dir, with the human's OK | — |
| **M1** (fixes) | m0024 runs after identity stores are attached and stops using registry `identity_id`; first sighting is recorded as `adopted`; delete the misattributed rows by exact id | — |
| **M2** (UI) | Global Memory tiles and full view; the editor pinned to the bottom on all three surfaces; Personal Memory editing in the Armory; Global Memory history UI | — (independent of storage) |
| **M3** (record) | the `agent-uid:<uid>:memory` record; reconcile at spawn; capture drift into the record; tables become caches; segment-proven adoption plus the human-confirmed adoption list; opt-out | M1 |
| **M4** (Global Memory global) | the `account:global-memory` record with stable entry ids; channels cache it; `memory_id` in the global definition record | M3 |
| **M5** (cloud sync) | cloud routes, S3 bodies, `memory_updated` wake; desktop push/pull for Global Memory first, then memory groups for Personal Memory | M3, M4 |
| **M6** (other providers) | project the record into Gemini's memory file; Codex and Kimi when they have one | M3 |

**Compatibility:**
- Older builds never read the new zones, and keep using their tables.
- Two builds of one channel running at once each reconcile from the same
  record; appends are serialized by the filestore.
- A build older than M3 that writes memory has its files captured as
  `source: provider` the next time an M3 build spawns that agent.

---

## 4. Open questions

1. **Adoption for pre-segment agents** (§2.1): is a human-confirmed list
   enough, or should the agent's working dir alone be trusted as proof?
   Recommended: human-confirmed. The working dir is shared by same-named
   agents across builds.
2. **Conflict files on disk** (`*.agentmux-conflict-*.md`): acceptable in
   the provider's folder, or keep conflicts only in the Armory?
   Recommended: on disk, so the agent itself sees and resolves them.
3. **Editing Personal Memory in the Armory** reverses a previous
   decision (`…CONTENT_VIEW` §6). Confirm.
4. **Sync scope:** Global Memory and memory groups only, or also
   per-agent ABF bundle instructions (empty today)?
5. **Memory groups across accounts** (sharing an agent's memory with
   another person): out of scope for v1?
6. **Retention of the record:** keep every version (like continuity), or
   the current "at least 50 versions / 90 days" rule?
7. **Where Global Memory lives in the Armory**: a code comment says it
   moves to the Bundles tab in the naming-consolidation spec's phase 4.
   Keep it under Memory (recommended, now that it matches Personal
   Memory).

## 5. Tests (summary)

**Record and reconcile:**
- A new account dir is filled from the record at spawn.
- A file the provider writes is captured as `source: provider`.
- Both sides changed → a conflict sibling; nothing is lost.
- A tombstone removes the file.
- A working-dir change projects memory into the new project folder.
- A new local channel keeps history (no `external_fs_write`
  mislabelling).
- Two concurrent builds reconcile to one head.
- The opt-out keeps today's behaviour.

**Adoption:**
- Only segment-recorded dirs are adopted automatically.
- A registry `identity_id` is never used.
- The human-confirmed list imports only the chosen dirs.

**Global Memory record:**
- Stable entry ids; the same content in two channels.
- A name clash shows as "(from <instance>)".

**Sync:**
- Fast-forward, three-way merge, and conflict sibling.
- Tombstones sync.
- Bodies over 400 KB go through S3.
- The account-scoped wake reaches only the account.
- An unlinked agent never syncs; a linked group does.
- System-tier entries never sync.

**UI:**
- Tiles render for Global Memory, including the CLAUDE.md, Combined and
  Add tiles.
- A tile expands to the full view with a breadcrumb.
- The editor fills the bottom region, and the actions, history and diff
  sit above it.
- The resize handle works and is remembered.
- Ctrl/Cmd+S saves.
- Personal Memory can be edited in the Armory.
- The Stash drawer's layout matches.
