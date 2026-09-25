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

**Second review** (1 P1, 8 P2, 8 P3), all accepted:
- **Shared directories:** a directory is treated as shared unless it is
  proven exclusive — machine-wide claims, registry vetoes, and files
  found at first sighting going to the adoption list (§2.1.2).
- **Conflicts:** they now converge (`conflicts_with`), and
  "disk equals head" is the first reconcile rule (§2.1.3).
- **Reconcile timing:** it runs before the provider process starts
  (§2.1.3).
- **Projection state:** keyed by directory, not by instance.
- **Reads:** sized from the database inside a read transaction (§2.1.1).
- **Quarantine:** quarantined versions never become heads.
- **Instances:** enrolled at the relay through the consent flow (§2.2).
- **Residual:** stated plainly, and agent-written Global Memory is
  quarantined by default when it arrives by sync (§2.2).
- **Imported history:** rows from unproven sources are imported only
  when proven (§2.1.1).

**Third review** (1 P1, 5 P2), all accepted:
- **First sighting:** the agent's own spawn directory, once proven
  exclusive, is adopted **automatically** as the baseline, index and
  topic files together, with an undo. Human adoption remains only for
  other accounts' directories (§2.1.2, §2.1.4).
- **Vetoes:** computed from machine-wide sources, counting only a
  **different** UID, and ignoring retired agents.
- **Claims:** released when the claiming agent is retired or deleted,
  and by a human action.
- **Drift:** re-checks the claim before every capture.
- **Reconcile:** has a 1 s budget and never blocks a spawn.
- **History import:** runs per agent, at its first M3 spawn.

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
  sha256 and whether it is deleted; per `(dir_id, file)`, the version last
  projected there (§2.1.3). It is rewritten after each append,
  under the same transaction.

Readers use `heads.json` for current state and read `log.jsonl` only for
history, so no tail window has to hold every file's head (the 64 KB
windows in continuity would miss rarely edited files).

**Reads are sized from the database, inside a read transaction.** Today
`read_file` sizes its read from the per-process `stat()` cache
(`core.rs:538 → 395-403`). After another process rewrites `heads.json`,
that returns truncated or overlong bytes. So:
- every read of `heads.json` and `log.jsonl` is sized from the database,
  inside a `read_txn`;
- the conditional append reads `heads.json` from its own `tx`, because
  calling `read_file` inside `write_txn` would deadlock on the connection
  mutex (`core.rs:189, 553`);
- both files are dropped from the cache (`forget_cached`) after every
  commit.

**Ordering within an append:**
- The blob is written before the log line, or in the same transaction.
  A log line never points at a missing blob.
- A `projected` event is appended only **after** the file write
  succeeds.
- `projected` events and `dir_id` values contain local paths, so they
  **never sync** (§2.2).

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

**Existing history is imported per agent, at its first M3 spawn.** Proof
of exclusivity only exists once the spawn has claimed its directory
(§2.1.2), so the import can't run earlier. At that spawn, after the claim
and before the baseline is adopted, `db_agent_native_memory_versions`
rows for that agent (from every channel store on the machine) are copied
into the record, deduplicated by sha256. Only then does the per-channel
table become a cache.

Rows with the sources `agent_inferred` (from m0024) and
`external_fs_write` (from drift) came from the registry-based
`list_all_memory_targets` on **every** user's machine (`m0024…rs:132`),
so they are treated as **unproven**:
- they are attached as **ancestors** of the baseline (§2.1.2) when their
  hash matches a file in the agent's own proven directory;
- otherwise they go to the adoption list (§2.1.4).

On this machine all 50 history rows are of these two kinds.

The rows known to be misattributed on this machine (§1.3) are deleted by
exact id in M0, by hand.

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

**Shared directories: shared unless proven exclusive.** Two agents can
share one memory directory: the same account plus the same working
directory, or agents with no linked account in `shared/providers/claude`.
On this machine a `-home-yas` project folder exists under 7 accounts.
Blank-`working_directory` agents launch in `~/.agentmux/agents/<slug>`,
which the code already treats as shared across same-name tabs
(`agent_open.rs:959-964`, `autoMemoryEnabled=false`). Only one agent here
has segments yet, so segment history alone can't detect sharing.

A directory is **exclusive** to an agent only when every one of these
holds:
1. **It isn't a known shared location:** not the blank-working-dir or
   `default_agent_working_dir` path, and not `$HOME` or an ancestor of
   another agent's working dir. Unlinked agents use
   `shared/providers/claude/projects/<cwd>/memory`, which is still one
   folder per working dir (`native_memory_handlers.rs:63-72`). So that
   location is shared only when **another unlinked agent has the same
   working dir**, not in general.
2. **No veto from a different UID.** No agent with a **different UID**
   could resolve to the same `dir_id` through its linked account × its
   working dir. The check uses machine-wide sources only:
   - the definitions in `shared/agents/definitions/<defId>.json`, which
     carry `working_directory`, excluding `retired/`;
   - the agent-to-account links in `shared/identity-store.db`;
   - the registry, excluding retired records.

   It does **not** read every per-version `objects.db`: there are 23
   here, some with older schemas, and their stale rows are never
   cleaned. Rows and records that carry the **same** UID (stale
   `identity_id`s, blank working dirs in old channels) never veto. These
   lookups **veto** only; they are never used to *find* a directory. The
   veto is precomputed into an index, rebuilt when definitions or links
   change, never scanned at spawn.
3. **It holds a machine-wide claim.** Before reconcile, the agent writes a
   claim to zone `memory-dir:<sha(dir_id)>` with the conditional append,
   and re-reads it afterwards. A live claim by a second UID makes the
   directory shared. Two agents spawning at once both see both claims.
   - **Drift capture re-reads the claim before every capture.** An agent
     that claimed first and was already running learns about a later
     claimant within one sweep, and stops capturing.
   - **Claims are released** when the claiming UID is retired or
     deleted: its definition moves to `retired/`, or the deleted-agent
     gate from #3591 applies. The Armory also offers a human "release
     this folder" action.

     This matters in practice: `buildInstanceSlug` adds only month, day
     and hour (`instance-slug.ts:60,73`), so re-creating a same-named
     agent within the hour, or on the same date a year later, reuses the
     working dir.
   - A UID never changes for an existing agent. M4d re-keys signing keys,
     not `db_agents.id`.

Measured on this machine: the three real agents each have their own
working dir. Two share an account, but their working dirs differ, so
their memory dirs differ. No veto fires, and all three can be proven
exclusive.

- **Detecting it:** a directory that fails any of the three is
  **shared**.
- **What a shared directory does not get:**
  - no projection, so the record's files aren't written into it;
  - no capture, so its files aren't attributed to either agent;
  - no deletion.
- **What it does get:** the Armory flags it as "Memory folder shared with
  <other agent>" (or "may be shared"), with a fix: give the agent its own
  working directory. The agents keep working exactly as today in the
  meantime.

**First sighting: the agent's own directory becomes the baseline.** When
the agent's **own live spawn directory** passes all three checks, its
current contents are adopted automatically as the baseline:
- **as one set:** the MEMORY.md index and every topic file together, so
  an index never points at missing files, and no topic is left without
  its index line;
- **labelled:** source `adopted`, detail "first sighting, own spawn dir";
- **reversible:** the Armory offers an undo, which removes the baseline
  from the record and leaves the files untouched;
- **with its history:** unproven history rows whose hash matches a file
  are attached as its ancestors (§2.1.1).

The baseline is skipped for a file whose hash already sits in **another
UID's** record. That file goes to the adoption list instead.

This carries no more risk than capturing later writes: any writer that
shares the folder after the claim would be captured anyway, so refusing
files that were there first protects nothing.

**Human adoption** (§2.1.4) remains only for other accounts' directories.

**Agents without segments yet** (every agent except one on this machine):
- `MemoryRead` and `MemoryWrite` resolve the directory from the caller's
  **live** spawn (its segment, which is written at spawn);
- until a segment exists, the Armory shows the registry-derived
  directory **read-only**, marked "unverified";
- drift capture skips that agent.

#### 2.1.3 Reconcile

**Where it runs:** in `persistent/spawn.rs`, **before the provider process
starts** (`cmd.spawn()`, `:165`), within a **1 s budget**, and it never
blocks a spawn.
- `spawn_process` is synchronous, and runs inline from `queue.rs:577`,
  `eager_resume.rs:234` and `resume_retry.rs:432`, while holding the
  resume-stripe mutex (`spawn.rs:111-121`).
- The filestore's busy timeout is 5 s (`core.rs:74`).

**If the budget runs out**, the spawn proceeds without projecting, and
records "reconcile deferred". The drift sweep finishes the work later. A
partial pass never writes tombstones. `config.env_vars` and
`config.working_dir` are already known there. It runs once per process
spawn, before Claude reads MEMORY.md, so a new account or a new working
dir starts its first session **with** its memory. The segment write stays
where it is (`:488`).

It doesn't run in `inject.rs`, whose account branch is skipped for agents
without an account, and which runs on every turn.

**Projection state is per directory.** The last-projected version is kept
per `(dir_id, file)` — a fact about the machine, not the channel. So two
channels (two WAN instances) on one machine never disagree about the
same directory. The instance is used only for sync provenance.

**For each file**, comparing the record's head, the version last
projected into this `dir_id` (from `heads.json`), and what is on disk:

**Rule 0: if the disk's sha256 equals the head's sha256, record
`projected` and do nothing else.** This covers a crash between writing a
file and recording it, and another channel having already projected the
same head.

| Disk vs last projected | Record head vs last projected | Action |
|---|---|---|
| same | same | nothing |
| same | changed | write the head to disk; record `projected` |
| changed | same | **capture**: a new version whose parent is the **last projected** version; record `projected` |
| changed | changed | **conflict** (below) |
| absent, never projected | exists | write the head (new account, cwd, channel or host) |
| absent, previously projected | exists | the provider deleted it: capture a **tombstone** whose parent is the last-projected version |
| exists, never projected | none | the directory is exclusive: this is the **baseline** at first sighting, or a capture afterwards (§2.1.2) |
| exists, never projected | exists (different sha) | a head from another directory meets a file here (e.g. after M0): **conflict** (below), never a silent overwrite either way |

**The parent is always the version last projected into that directory**,
never the record's current head. A version that arrived from another
directory in the meantime therefore shows up as a conflict instead of
being overwritten.

**Conflicts:**
- **The disk version D becomes the head**, carrying a `conflicts_with: H`
  marker, where H is the record's previous head. The last-projected
  version becomes D, so the next reconcile doesn't raise the conflict
  again, and D is never overwritten.
- The other version H is written beside it as
  `<stem>__conflict_<short>.md`. The stem is truncated so that the whole
  name stays within `validate_filename`'s 200-character limit and stem
  rule (`:437, 449-453`), so the agent can read, merge and remove it with
  its normal memory tools.
- Both the conflict file and the index line are recorded as `projected`
  in the same transaction. `__conflict_*` files are **never captured** as
  provider writes.
- A merge (by the agent, or in the Armory) records a version with **two
  parents** (D and H), which clears the marker.
- The **MEMORY.md index** gets one line pointing at the conflict file.
  Claude loads only the index, so an unindexed file would never be seen.
- The Armory shows the conflict with a merge view.

**Tombstones.** A deleted head removes the file from disk **only if** the
disk's sha256 equals the tombstone's parent. Otherwise it is a conflict.

**Drift capture while the agent runs.** The drift detector
(`native_memory_drift.rs`) keeps watching the directory. It captures into
the record with the same rule, using the directory's last-projected
version as the parent. Before every capture it **re-reads the claim**
(§2.1.2).

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
  N accounts"). The list gets a server-issued `list_id`. **The client
  submits only `(list_id, index, dir hash)` choices, never a path.** A
  directory that changed after the list was issued is refused, and the
  human sees a refreshed list.
- The agent's own spawn directory is adopted automatically as the
  baseline (§2.1.2). **Other directories are never adopted
  automatically**, including ones older segments point to; they go on
  this list.

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
upload, and every receiver checks it again.

**The signature proves the instance, not the author.** Any agent on an
instance can read its key (WAN spec §4), so `source` stays a label the
sending instance asserts.

**Instances are enrolled at the relay, through the consent flow.** The
human confirms an instance in the system browser, with
re-authentication, on a page showing its full 26-character id. The relay
refuses uploads from instances that aren't enrolled. This does **not**
depend on the host-gated window, so M5 doesn't wait for the
GHSA-6726-q276-g6f6 fix.

**Quarantine.** Two kinds of version are quarantined:
- versions from an instance **this receiver** hasn't accepted yet;
- by default, **agent-authored Global Memory versions** (`source` other
  than `armory-ui` or `human`) from any other instance, until a human
  accepts them.

Quarantined versions sit in a **pending set, outside `heads.json` and
outside the `db_bundles` cache**. So they are never projected into a
folder, and never rendered into CLAUDE.md by `format_global_bundle_block`
(`agent_open.rs:992`). They show in the Armory with Accept/Reject.

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
  scope)`.
  - The number and the item are written in one `TransactWriteItems`, so
    number N+1 never becomes visible before N.
  - Receivers dedupe by version id.
  - Desktop clocks are never trusted for ordering.
- **Storage:** DynamoDB holds the event metadata. **Bodies go to S3** at
  `s3://…/<account>/<sha256>`, prefixed by account.
  - Uploads use **presigned S3 PUTs** with a sha256 checksum, because the
    relay Lambda's roughly 6 MB body limit is below the 10 MB per-file
    cap.
  - Downloads are presigned GETs, issued only after the account check.
- **What never syncs:** `projected` events and `dir_id` values, which are
  local paths.
- **Wake** (dependencies listed in §3):
  - `broadcast.ts` gains a payload parameter; today it is hard-coded to
    `inject_available` (`:65`);
  - `ws-connect.ts:74-81` stores the account for user tokens too — the
    integrations spec's I1 change;
  - the desktop's `ServerMsg` enum (`cloud_subscriber.rs:112-128`) gains
    `MemoryUpdated`.

**Residual, stated plainly.** Memory is loaded as instructions: MEMORY.md
into every Claude session, Global Memory into CLAUDE.md.

**Personal Memory in a group.** Any agent on an enrolled instance can
write its own memory file, and that file is captured, synced, and
projected on the linked machines. This is inherent and accepted: it is
the same logical agent's memory reaching its own counterpart, exactly as
if it had written its MEMORY.md there itself.

**Global Memory.** An agent's `GlobalMemoryWrite` would reach **every
agent on every machine** of the account. That is why agent-authored
Global Memory is quarantined by default on receipt (above). An operator
who turns that off accepts the reach.

**Source labels.** Until the GHSA-6726-q276-g6f6 fix, and until the
instance key is kept out of agents' reach (WAN spec open question 5), an
agent can forge `source` labels on its own instance. Receivers must
treat `source` as advisory.

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
surface **as a fraction**. The column has a definite height (the pane),
so the `50%` floor resolves. Small panes (down to 128 px) keep working
because of that floor.

**Unsaved edits are never lost.** Today the detail panel remounts on
every `agent:memory:changed` for that agent (`native-memory-manager.tsx:344, 580`).
With a dirty draft, a change event instead:
- shows a "changed since you started editing" banner, offering View
  change or Keep editing;
- keeps the draft.

A save carries its base version, and a base that has moved becomes a
conflict (§2.1.3), not an overwrite.
- **The base version needs new RPC parameters.** `agent:memory:write_file`
  and the bundle save gain an optional `base_sha256` in **M2**; without
  it they behave as today.
- **The banner compares the file's hash.** It appears only when the
  hash differs from the draft's base, not on every change event
  (`refreshNonce` bumps on every refresh, `native-memory-manager.tsx:344`).

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
| **M0** (manual, with the human's OK) | Restore Maricon's memory. Stop Maricon first. The human confirms all three source accounts are Maricon's (`b43cec34` is the stale id from the m0024 bug). Write the **union** into `1b64d8a3`'s `projects/-home-yas--agentmux-agents-maricon-09101/memory` (empty today; only Maricon is linked to that account, and the folder is keyed by Maricon's working dir, so no other agent is affected): `ee8af73e` (newest, 3 files), plus the two topics only in `b43cec34`, with a unioned MEMORY.md. `d5ba7d63`'s older, superseded `agentmux-contribution-rules.md` goes to the backup only, never as an indexed file. Take a backup first. Also delete this machine's misattributed history rows by exact id. | — |
| **M1** (fixes) | stop using registry `identity_id`, the blank-working-dir fallback and name-derived dirs to *find* memory, in `list_all_memory_targets` and drift; MemoryRead and MemoryWrite resolve from the caller's live segment, and the Armory shows unverified dirs read-only (§2.1.2); **a new post-attach step** after `attach_identity_stores` replaces m0024's backfill (m0024 has already run in existing channels, so moving it wouldn't help), labelling first-seen files in the version table `adopted`, not `external_fs_write` | — |
| **M2** (UI) | Global Memory tiles and full view; history and content split; editors pinned to the bottom; dirty-draft protection; Personal Memory editing through the existing RPC; Global Memory history RPCs | — |
| **M3** (record) | the filestore conditional append and database-read sizes; the `agent-uid:<uid>:memory` record (log, blobs, heads); the veto index and claim zones with release; the per-agent history import and baseline at first M3 spawn; time-bounded reconcile before spawn; drift into the record with claim re-checks; server-listed, host-confirmed adoption for other accounts' directories, with index union; opt-out | M1 |
| **M4** (Global Memory record) | `global-memory:<scope>` with entry ids, order events and the import step; legacy-build capture; the bundle sidecar | M3 |
| **M5** (cloud sync) | the integrations spec's I1 `ws-connect` change and consent flow; the WAN spec's instance keys, with instances enrolled at the relay; relay routes, transactional sequence cursor, presigned S3 bodies, broadcast payload, `MemoryUpdated`; the pending set for quarantine; Global Memory first, then memory groups | M3, M4; integrations I1; WAN instance keys |
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
  same-user process can confirm adoption through the host channel.
  Adoption is limited to directories the server lists for that agent.
  Instance enrolment for sync uses the relay's consent flow, so the fix
  doesn't block it.
- **Any agent on an enrolled instance** can push its own Personal Memory
  to its linked counterparts, which is inherent. Agent-authored Global
  Memory is quarantined on receipt by default, and `source` labels are
  advisory (§2.2).
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
- Reads are sized from the database: a process reads another process's
  rewrite of `heads.json` correctly.
- A blob is written before its log line.
- `projected` is recorded only after the file write.
- Three srv processes capturing the same write produce one version.
- A process sees another process's appends (database-read size).
- `heads.json` stays consistent with the log after a crash between
  appends.
- History import is deduplicated by hash; the misattributed rows are not
  imported.
- The record being unavailable never fails a MemoryWrite.

**Directories:**
- Only spawn-env directories are used to find memory; registry
  `identity_id` and the blank-working-dir fallback are never used for
  that.
- These are treated as shared:
  - `$HOME`;
  - a blank-working-dir path;
  - a directory vetoed by a different UID;
  - a directory claimed by a second live UID;
  - two agents spawning at once.
- These are **not** vetoes:
  - the same UID's stale rows and registry records;
  - an unlinked agent under `shared/providers/claude` with a unique
    working dir.
- A retired or deleted UID's claim is released; the human release action
  works.
- Drift stops capturing after a second claim appears.
- A shared directory is neither projected nor captured.
- First sighting of the agent's own exclusive directory adopts the index
  and topic files together as one baseline, with ancestors attached.
  Undo works.
- A file whose hash is already in another UID's record goes to the
  adoption list.
- A new account dir is filled from the record.
- A working-dir change projects into the new folder.

**Reconcile:**
- Rule 0: after a crash between the file write and the record, there is
  no spurious conflict.
- Every row of the §2.1.3 table.
- A conflict converges: it is not raised again on the next sweep, and D
  is not overwritten.
- `__conflict_` files aren't captured; a long stem is truncated.
- Reconcile runs before the provider process starts, within its budget.
  A timeout still spawns and records "reconcile deferred". A partial pass
  writes no tombstones.
- Two channels on one machine share the projection state.
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
- An instance that isn't enrolled is refused at the relay.
- A new instance's versions, and agent-authored Global Memory, are
  quarantined.
- A quarantined version never reaches `heads.json`, `db_bundles` or
  CLAUDE.md.
- Sequence numbers are gap-free.
- Presigned uploads are used for bodies up to 10 MB.
- `projected` events and `dir_id` never sync.
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
