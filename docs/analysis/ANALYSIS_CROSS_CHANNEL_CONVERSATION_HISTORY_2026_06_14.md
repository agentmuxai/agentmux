# Cross-channel conversation history: why agents open empty, and how to fix it

**Date:** 2026-06-14 · **Author:** AgentX · **Status:** Analysis + implementation plan

## 0. TL;DR

After PR #1396 your agents from other builds/channels finally **appear** in "My Agents".
But opening one shows an **empty pane** — the conversation doesn't load.

The model is simple: **a conversation is the agent's data; the pane virtualizes it and loads
it when you open the agent** — exactly as it already does for an agent in its own channel. It
loads empty cross-channel for one reason: AgentMux stores that conversation **per-channel**
(`filestore.db`), so when you open the agent from a *different* build there's nothing local
to load.

**The fix is one thing:** store the agent's conversation **globally, per-agent**, and have the
open path load it from there. The conversation already *is* a blockfile, and the **O(1)
paginated/virtualized loader already exists** (#336–#342, ultra-long-sessions: `blockfile:read_range`
+ `line_count` fast-path) — so this is **storage + addressing only**, no new render path or
history format. Conversation history is the **last agent surface that's still per-channel** —
definitions, instances, workspaces, and auth all became global in #1387–#1396.

(Separately: actually *continuing* the conversation — sending a new message and having the CLI
remember — additionally needs the provider session reachable for `--resume`. That's a smaller
companion item, §6, not the main fix.)

---

## 1. Symptom (reproduced)

- "My Agents" now lists the 9 cross-channel agents (Smark, Naki, Mazs, CodexPo, Clamk,
  GeminiOpp, Smike, Claude×2) — ✅ PR #1396.
- Opening any of them → **empty pane**.
- Opening an agent created in the *current* build shows its conversation fine.

## 2. How the pane loads a conversation (today) — and it's already O(1)

The agent pane renders AgentMux's **own** transcript — the agent's **blockfile** in the
FileStore — written from the agent's stdout (`subprocess.rs:17`, `persistent.rs:20`: "process
stdout → .jsonl persistence + WPS blockfile events"). The FileStore is opened
**per-channel-version**:

> `agentmux-srv/src/main.rs:527` — `FileStore::open(<data_dir>/db/filestore.db)`
> → `channels/<ch>/versions/<v>/data/db/filestore.db`, keyed by `block_id`.

**Crucially, loading is already O(1).** The ultra-long-sessions work (#336–#342,
`docs/retros/2026-04-12-ultra-long-sessions.md`) made the blockfile a paginated, virtualized
store: `blockfile:read_range` (page the visible window) + `blockfile:line_count` O(1) fast-path
via `session:line_count` meta (`app_api.rs` read_range/line_count handlers), rendered with
`content-visibility` virtualization (`AgentDocumentView.tsx:208`). So a 400 KB+ conversation
already opens instantly — only the visible window is read.

**So the loader is not the problem.** "Open agent → `read_range` its blockfile" already works
and is O(1) — it just resolves the blockfile from **this channel's** `filestore.db`, so it
finds nothing for an agent that ran elsewhere.

## 3. Why cross-channel opens empty (root cause)

When you open a cross-channel agent from "My Agents", there is **no local block to load**:

1. The conversation node-graph lives in **another channel's** `filestore.db` (`main.rs:527`),
   keyed by a `block_id` this channel doesn't have — and with per-build channels now, that
   channel's data dir may have been pruned.
2. The cross-channel **registry record has no `block_id`** and no source-channel pointer
   (`agentmux-srv/src/registry/schema.rs:34-67` — fields are instance_id, instance_name,
   definition_id, identity_id, memory_id, session_id, working_dir, source_agents_base,
   timestamps, versions). So there's nothing telling the open path *what* to load or *where*.

Net: the reattach spins up a **fresh, empty block** → empty pane. The data isn't lost — it's
just stored somewhere this build can't see.

**Evidence the data exists:** the rendered node-graph is in the origin channel's `filestore.db`,
and the raw provider transcript is also intact and **global** at
`~/.claude/projects/<slug(cwd)>/<sid>.jsonl` (Mazs: `…mazs-0527n/2178ee0f-…jsonl`, 421 KB).

## 4. The one fix: globalize the agent's conversation; load it on open

### Key discovery (from the code, two deep-dives)

The right abstraction **already exists** — we don't invent a store, we make an existing one global:

- **The `agent:<defId>:current` zone.** AgentMux already stores an agent's transcript in a
  FileStore *zone* keyed by `definition_id` (block meta `agentId`), not just `block_id`. Because
  "New from template" forks a **per-agent definition**, this zone is effectively **per-agent**.
  Helpers: `agent_session::agent_current_zone(def_id)` → `"agent:{def}:current"`
  (`agent_session.rs:77-83`).
- **A snapshot fast-path already restores the pane cross-*block*.** On open,
  `useHistoryPagination.ts:195` calls `AgentSessionReadCommand({definition_id})`, which reads
  `output.state.json` from that zone and `HistoryRestored` — *that's* why same-channel reattach
  shows history. It's empty **cross-channel** only because the zone lives in the **per-channel**
  `filestore.db` (`main.rs:527`).
- **The blockfile is append-only and `append_data` is O(append-bytes)** (`filestore/core.rs:436`).
  Every agent-output path funnels through ONE write-through: `handle_append_block_file` →
  `fs.append_data(block_id, "output", line)` at **`shell.rs:1271`** (callers `persistent.rs:428`,
  `subprocess.rs:664`, `acp.rs:405`).
- **The read handlers key purely on `block_id`** (`app_api.rs` `blockfile:read_range` `:1029`,
  `blockfile:line_count` `:957`); they can map `block_id → defId` via block meta `agentId`
  (`app_api.rs:981` already does this lookup).

So: **back the `agent:<defId>:current` zone with a GLOBAL FileStore** (write there, read there).
The renderer, the RPCs, and the frontend are all unchanged.

### Implementation plan (recommended: Option A — global-zone fallback)

| # | Change | File (evidence) |
|---|---|---|
| **1. Global FileStore (foundation)** | `pub fn resolve_shared_transcripts_dir()` → `<shared>/agents/transcripts` (sibling of `resolve_shared_registry_dir`); open `FileStore::open(<dir>/filestore.db)` near `main.rs:527` as `Option<Arc<FileStore>>`; add `AppState.global_transcript_store` (`server/mod.rs:50`). 2nd FileStore is safe (independent SQLite/WAL). | `registry/paths.rs:25-42`, `main.rs:514-527`, `server/mod.rs:50` |
| **2. Mirror the `output` NDJSON globally** | At `shell.rs:1271`, after the per-channel `append_data`, ALSO `append_data(defId, "output", data)` into the global store (same bytes, fire-and-forget). Resolve `defId` **once** at controller construction from block meta `agentId` / `wstore.instance_get_active_for_block` (`agents.rs:1739`) — never per line. | `shell.rs:1205-1294`, controllers `blockcontroller/mod.rs:364-401` |
| **3. Read fallback on open** | In `blockfile:read_range` + `line_count`: when the per-block `output` is empty, map `block_id → defId` (block meta `agentId`, `app_api.rs:981`) and read the **global** store's `agent:<defId>:current` zone. Guard to `filename == "output"`. Zero frontend change. | `app_api.rs:957-1136` |
| **4. Snapshot too (cheap win)** | Point `AgentSessionWrite/Read` (`output.state.json` snapshot) at the global store as well, so the existing snapshot fast-path restores cross-channel immediately even before NDJSON paging. | `agent_session.rs` write_session_state/read |
| **5. Backfill the 9** | One-shot: for each migrated agent, populate the global zone's `output` from the provider `.jsonl` (`ClaudeHistoryAdapter` already normalizes + parses — `claude_adapter.rs`, `history/index.rs`) or the origin channel's blockfile. Mirrors the #1391/#1393 one-shot. | new `migrate`-style one-shot |
| **6. Tests + docs** | Round-trip (append→global→`read_range`), backfill maps the 9, cross-channel open pages N lines. Update `SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md:57`. | — |

**Why Option A over "copy on reattach":** read keys are already `block_id`; the block already
carries `agentId`; FileStore is the same zone-keyed store — so the global read is one extra
`read_file(zone, "output")`. No per-reattach byte copy, no launch-saga race window, and it also
covers the no-snapshot / provider-only case. (Confirmed by the read-path deep-dive.)

**Smallest first slice (if we want to land value fast):** steps **1 + 4 + 5** (global FileStore +
snapshot read/write there + backfill snapshots) make opening a cross-channel agent restore its
conversation via the *existing* snapshot fast-path — **no hot-path change**. Steps **2 + 3** add
full O(1) pagination of the complete NDJSON history as the follow-on.

**Why this is the right shape:** same move we already made for definitions, instances,
workspaces, and auth — promote a per-channel store to global per-agent and read it everywhere.
The O(1) loader (#336–#342) and the zone abstraction both already exist, so this is **storage +
addressing only** — no new render path, no new format.

---

## 5. What's already shipped that helps

- **The O(1) loader (#336–#342, ultra-long-sessions).** `blockfile:read_range` + `line_count`
  fast-path + virtualization already page huge conversations instantly. This fix reuses that
  verbatim — it only changes *where* the blockfile is read from. So the hard part (loading a
  400 KB+ conversation without stalling) is already solved.
- The **registry is populated** (#1391/#1393) and the **live mirror is fixed** (this branch),
  so the agent set + `session_id` (for new agents) are current.
- `ClaudeHistoryAdapter` already finds an agent's transcript by cwd across both global
  `~/.claude/projects` and AgentMux's isolated homes (`claude_adapter.rs:29-72`) and reads the
  `session_id` off the filename (`:205-209`) — directly reusable for the backfill (step 4).
- `import-agents.sh` is a working reference for locating the best transcript for an agent
  (`best_session_for_slug :75-101`) and the cwd→project-dir slug (`slugify_cwd :69`).
- `SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` P0/P1a are done (`:150-153`).

## 6. Companion item — continuing the conversation (smaller, separate)

Loading the history (§4) makes the pane show the conversation. To also let the user **send a
new message and have the CLI remember**, the provider session must be `--resume`-able:

- The 9 migrated records have `session_id = null` (the #1393 migration set it; the live mirror
  *does* capture it for new agents — `registry_mirror.rs` `empty_to_none(&inst.session_id)`).
  Recover it from the transcript filename during the §4 backfill.
- AgentMux runs Claude with `CLAUDE_CONFIG_DIR=~/.agentmux/shared/providers/claude`, so
  `--resume` reads the isolated home; the migrated transcripts are in global `~/.claude`. Copy
  the chosen `.jsonl` into the isolated home before resume — `rehydrate_claude_session`
  (`import-agents.sh:107-120`), already done for the import path (spec P0).

This is provider-specific and *optional* for the "I can see my history" goal; it's required for
the "I can keep talking and it remembers" goal. Fold it into the §4 backfill (recover `sid`)
plus a resume-time rehydrate.

## 7. Recommendation

Do **§4 (globalize transcript + load on open)** as the feature — it's the single coherent fix
that matches your model ("open the agent, the history loads") and finishes the cross-channel
arc. Bundle the §6 `session_id` recovery into the same backfill so continuing also works, and
add the resume-time rehydrate as a small follow-on if needed.

## 8. Open questions for the owner

1. Canonical store format for step 1 — AgentMux's WPS node-graph (`output.state.json`, what the
   pane renders directly → cheapest to load) vs the provider `.jsonl` (what `--resume` needs).
   Likely keep the node-graph for display + the `.jsonl` for resume; avoid translating.
2. Should **memories** (`db_memory_bundles`, the other per-channel holdout) be globalized in the
   same effort, so an agent fully follows you across builds?
3. Keying — by `instance_id` (stable, in the registry) vs by `(definition_id, instance_name)`?
   `instance_id` is unambiguous; the latter survives a re-create but can collide.

---

## 9. Implementation status (PR `agentx/global-transcript-store`)

Steps **1–4 are implemented** and shipped in this PR; step **5 (backfill of the 9) is deferred**
to a verified follow-up. A `#1361` interaction reordered the dependency chain (see below).

### What landed (1–4)

| # | Change | Key code |
|---|--------|----------|
| **1** | `resolve_shared_transcripts_dir()` → `<shared>/agents/transcripts`; second `FileStore` opened in `main.rs` as `Option<Arc<FileStore>>`; `AppState.global_transcript_store`; process-global handle `set/get_global_transcript_store` (so the controller hot path reaches it without threading). | `registry/paths.rs`, `main.rs` (after the per-channel store), `server/mod.rs`, `backend/agent_session.rs` |
| **2** | Hot-path mirror: `handle_append_block_file` gains `global_output_zone`; mirrors the agent `output` stream into the global `agent:<defId>:current` zone. Zone resolved once per reader from block meta `agentId` (`resolve_global_output_zone`). Wired in subprocess / persistent / acp readers. PTY `term` never mirrors. | `blockcontroller/shell.rs`, `persistent.rs`, `subprocess.rs`, `acp.rs` |
| **3** | Read fallback: `blockfile:read_range` + `blockfile:line_count` read the global zone when this channel has no local `output` for the block (`global_output_source`, `global_zone_line_count`). Zero frontend change. | `server/app_api.rs` |
| **4** | Snapshot overlay mirrored to + read-fallback from the global store (`write_session_state` / `read_session_state`). Additive — per-channel stays primary, so same-channel behaviour is byte-identical. | `backend/agent_session.rs` |

Tests: 9 new unit tests (zone resolution, hot-path mirror round-trip, read/line_count fallback,
snapshot read fallback), full suite green (1128 passed).

### The `#1361` correction (why the order changed)

The analysis's "smallest first slice = **1 + 4 + 5**" predates **PR #1361**, which made the
snapshot a *lightweight overlay* (no `nodes[]`; just `highWaterMark` + `documentState`). The pane
now reconstructs history by reading the `output` NDJSON via `read_range(0…highWaterMark)`
(`useHistoryPagination.ts`). So globalizing the **snapshot alone no longer restores anything** —
the content lives in `output`. The load-bearing core is therefore **1 → 2 → 3**.

Further, the v2 snapshot fast-path is gated on `sourceBlockId === opts.blockId`. A cross-channel
open creates a **fresh local block** whose id never equals the origin block's `sourceBlockId`, so
the frontend falls through to **Path B** (line-count → read_range). Path B gates on
`BlockfileLineCount === 0` → empty pane. That's why **`line_count` had to gain the global fallback
too** (not just `read_range`) — it is the cross-channel path.

### Deferred — step 5 (backfill the 9) + §6 `session_id` recovery

Go-forward cross-channel history works without backfill (the Phase-2 live mirror writes the exact
correct format). Backfilling the **existing** 9 is deferred because it's a **run-and-verify** task,
not a write-blind one: the global `output` must be **raw provider stream-json** (one JSON/line, the
format the frontend translator + `parseHistoryLines` expect), but the backfill *source*
(`~/.claude/projects/<slug>/<sid>.jsonl`) is the provider's **session transcript**, whose envelope
is related to but **not byte-identical** to stream-json stdout — and `ClaudeHistoryAdapter` yields a
*lossy summary*, so it can't be replayed directly. Landing it safely requires confirming the real
transcripts render in the app. Tracked as a follow-up (fold in §6 `session_id` recovery there).
