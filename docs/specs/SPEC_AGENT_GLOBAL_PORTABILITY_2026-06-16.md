# SPEC: Globally Portable Agents — Final Implementation

**Date:** 2026-06-16
**Author:** AgentX
**Status:** Final — supersedes per-incident patches in #1472, #1479, #1486

---

## Problem Statement

AgentMux is a multi-build, multi-channel desktop application. Every `task package` build
produces a channel like `local-main-b28b7a-a671289f`; every branch's `task dev` instance
runs as `dev/main`. An agent created in one channel must work identically in every other
channel — same history, same session continuity.

The current implementation has three data stores for agent state. Channel-local identifiers
leak into the global store. On every version bump or new build, agents appear blank or
restart fresh sessions. This is a recurring, escalating incident pattern:

- **#1399 / #1403** — global zone added; backfilled, but broke on first live run.
- **#1472** — normalize `sourceBlockId` on write to global; startup heal.
- **#1479** — backfill `session_id` from largest provider `.jsonl`.
- **#1486** — re-derive stale `highWaterMark` from global zone.
- **Backend: read-global-first** — per-channel snapshot shadowed the correct global one.

Each patch fixes one narrow failure mode and introduces no protection against the next.
This spec defines the architecture that makes agents first-class portable objects so no
future version bump requires data surgery.

---

## Data Zone Model (Current State — The Problem)

```
~/.agentmux/
├── shared/
│   ├── agents/
│   │   ├── registry/        ← Agent definitions (global)
│   │   └── transcripts/     ← Blockfile store (global)
│   │       └── filestore.db   SQLite: zones = "agent:<defId>:current"
│   └── providers/
│       └── claude/
│           └── projects/    ← Provider session .jsonl files (external)
│
└── channels/
    └── local-main-b28b7a-a671289f/
        └── transcripts/
            └── filestore.db ← Per-channel blockfile store
                               Has: snapshot with sourceBlockId="real-local-block-id"
                               THIS IS THE BUG VECTOR
```

### Why This Breaks

`write_session_state` writes to **both** stores:
- Per-channel: snapshot with `sourceBlockId = "658d3caf"` (real local block)
- Global:      snapshot with `sourceBlockId = ""` (normalized — correct)

`read_session_state` read per-channel **first** (pre-fix). Per-channel won. On a
cross-channel open, `v2SameBlock` fails (foreign block). NDJSON fallback runs. User
sees the last 200 lines instead of full history, or blank if local output is empty.

---

## Target Architecture

### Principle 1: Agent state is globally owned, locally cached

All persistent agent state MUST live in `~/.agentmux/shared/`. Per-channel stores are
**non-authoritative caches** — their absence must never cause data loss, and their
presence must never override the global record.

```
~/.agentmux/shared/agents/
├── registry/
│   └── <agentId>.json        ← Definition + session_id (authoritative)
└── transcripts/
    └── filestore.db          ← Blockfile: all conversation output + snapshots
        zone pattern: agent:<defId>:current
```

### Principle 2: No channel-scoped identifiers in global artifacts

The global snapshot MUST be anchored to the **agent identity** (`sourceBlockId: ""`), never
to a channel-local block. Any field that is only meaningful in the writing channel must be
stripped before persisting to the global store.

Formally:
> **Invariant G1:** `global_snapshot.sourceBlockId === ""`  at all times.

### Principle 3: Per-channel data is subordinate

When both global and per-channel snapshots exist, the global snapshot is authoritative.
Per-channel data is read only when global data is absent (old pre-global-store builds).

Formally:
> **Invariant G2:** `read_session_state` returns the global snapshot when present,
> regardless of per-channel state.

This is the fix landed in this session (commit on branch `agentx/fix-agent-read-global-first`).

### Principle 4: Schema versioning — no breaking changes without negotiation

Every serialized agent artifact MUST carry a `schemaVersion` integer. Readers MUST
ignore unknown fields (forward compat) and MUST handle old versions (backward compat)
by applying defaults. Version bumps require:
1. An increment of `AGENT_DATA_SCHEMA_VERSION` in `agentmux-common`.
2. A migration registered in the startup sequence.
3. The migration MUST be idempotent.
4. Old builds that do not know the new version MUST still be able to read the artifact
   (unknown fields passed through unchanged; version-gated behavior defaults to the
   old path).

---

## Snapshot Schema — Versioned Contract

### Current (schemaVersion 2)

```json
{
  "schemaVersion": 2,
  "sourceBlockId": "",
  "highWaterMark": 83709,
  "savedAt": "2026-06-16T01:21:26Z"
}
```

**`sourceBlockId: ""`** is the portable anchor. It tells the frontend:
> "I was last written with `highWaterMark` lines in the agent zone. When restoring,
> find the current block, ask for its line count from the agent zone, and read the
> appropriate window."

### schemaVersion 3 (this spec introduces)

Adds `sessionId` inline so snapshot + session continuity are co-located:

```json
{
  "schemaVersion": 3,
  "sourceBlockId": "",
  "highWaterMark": 83709,
  "sessionId": "91f26930-8100-4ec1-8d4d-c3b580f2f688",
  "savedAt": "2026-06-16T01:21:26Z"
}
```

- `sessionId` is the provider (Claude) session ID for `--resume`.
- Writing: captured by `hydrate_session_id_from_config` after each turn; included in
  `write_session_state`.
- Reading: preferred over the registry's `session_id` (snapshot is more recent).
- Backward compat: missing `sessionId` → fall back to registry → then backfill scan.

---

## Registry Schema — Versioned Contract

### Current

```json
{
  "id": "f81f9785-...",
  "name": "Naki",
  "prompt": "...",
  "session_id": "91f26930-...",
  "dataVersion": 1
}
```

### No change needed in registry for this spec

The registry's `session_id` is the fallback source of truth. With schemaVersion 3
snapshots, the snapshot carries `sessionId` inline, making the registry field
redundant for non-backfill paths. The registry field STAYS (backward compat with
pre-v3 readers) but is written less eagerly.

---

## Read Path — Final State

```
read_session_state(filestore, definition_id):
  1. Try global filestore (shared/agents/transcripts/filestore.db)
     zone = "agent:<defId>:current"
     → found: return (snapshot_json, saved_at)   ← PREFERRED (G2)
  2. Fallback: try per-channel filestore
     → found: return (snapshot_json, saved_at)   ← legacy only
  3. None: return (None, None)
```

Frontend `useHistoryPagination.ts` restore path:

```
v2SameBlock = schemaVersion >= 2
              && highWaterMark > 0
              && (sourceBlockId === "" || sourceBlockId === currentBlockId)

If v2SameBlock:
  hwm = snapshot.highWaterMark
  if sourceBlockId === "" && sourceBlockId !== currentBlockId:
    // agent-anchored: re-query live line count to correct any stale hwm
    liveCount = await BlockfileLineCountCommand(currentBlockId)
    hwm = max(hwm, liveCount)
  windowStart = max(0, hwm - RESTORE_WINDOW_LINES)
  read_range(currentBlockId, windowStart, RESTORE_WINDOW_LINES)
  // global_output_source fires here if local output is empty → reads global zone ✓
```

---

## Write Path — Final State

`write_session_state(filestore, definition_id, snapshot_json)`:

```
1. Parse snapshot_json → Snapshot struct
2. Validate: sourceBlockId may be "" or a real block id (caller's value)
3. Write to per-channel filestore AS-IS (for same-channel same-build restore perf)
4. Normalize: strip sourceBlockId → ""
5. Write normalized to global filestore
6. Assert: global_snapshot.sourceBlockId === ""   ← enforce G1 in debug builds
```

The per-channel write is a **performance cache** only. It enables the fast path
(exact same block, no line_count RPC) for same-channel same-build restores. It is
never the authoritative source.

---

## Write Path — What MUST NOT be stored globally

| Field | Global store? | Reason |
|-------|--------------|--------|
| `sourceBlockId` (real) | ❌ NEVER | Channel-local; meaningless cross-channel |
| `sessionId` | ✅ YES | Provider-scoped, version-invariant |
| `highWaterMark` | ✅ YES | Position in agent zone (same across channels) |
| `schemaVersion` | ✅ YES | Required for forward/backward compat |
| `savedAt` | ✅ YES | Staleness detection |

---

## Startup Migration Sequence

Executed once per process start, idempotent:

```
1. heal_global_snapshot_source_block_ids()    ← already in #1472
   Rewrites any global snapshot with sourceBlockId != "" → ""

2. backfill_session_ids()                     ← already in #1479
   For each agent in registry with session_id == null:
     scan shared/providers/claude/projects/<agentSlug>/
     pick the largest .jsonl file
     write session_id to registry

3. migrate_snapshots_v2_to_v3()               ← NEW (this spec)
   For each agent with a v2 global snapshot:
     read registry session_id
     if present: rewrite snapshot as v3 with sessionId = registry.session_id
     mark schemaVersion = 3
```

Each migration MUST:
- Log its actions at INFO level (count of records healed)
- Be safe to run with other read-only instances running concurrently (SQLite WAL)
- Be safe to run when the global filestore is empty

---

## Cross-Channel Invariants

These invariants MUST hold after any future code change touching agent state:

| ID | Invariant | Checked by |
|----|-----------|-----------|
| **A1** | `global_snapshot.sourceBlockId === ""` for all agents, always | startup heal + write-path assert |
| **A2** | `registry.session_id` is set for all agents with any conversation | startup backfill |
| **A3** | Opening an agent in a channel with no prior local block renders full global history | e2e test (pending) |
| **A4** | The first message after a cross-channel open resumes the original session | e2e test (pending) |
| **A5** | No snapshot field contains a channel/build/process-local identifier | code review gate |
| **A6** | Deleting `~/.agentmux/channels/*/` does not lose any agent data | recovery test (pending) |

---

## Portability Contract for Agents (Public-Facing Guarantee)

An agent is **globally portable** when:

1. **Its definition is in `shared/agents/registry/`** — not tied to any build or channel.
2. **Its conversation history is in `shared/agents/transcripts/`** — the global zone
   `agent:<defId>:current` is the authoritative record.
3. **Its session ID is in its snapshot (v3) or registry** — so any channel can `--resume`
   the same provider session.
4. **Its snapshot `sourceBlockId` is `""`** — any channel can open it without knowing which
   specific block produced the history.

Given (1–4), the following operations produce correct behavior without data surgery:
- `task package` new version → open all agents → full history and session continuity ✓
- Machine wipe + restore `~/.agentmux/shared/` from backup → all agents intact ✓
- Multiple dev + portable instances running simultaneously → no cross-contamination ✓

---

## What Was Not Done Previously (And Why These Incidents Kept Recurring)

### Missing: G2 (read global first)

`read_session_state` read per-channel first until this session. Every write path
correctly normalized the global copy, but the very next same-channel open wrote the
real `sourceBlockId` to per-channel (correct for same-channel), and subsequent reads
returned that per-channel snapshot — undoing all the cross-channel work.

**Fix:** Swap read order. Global first, per-channel fallback. Done in this session.

### Missing: Invariant testing

No e2e test covered the "run in channel A, open fresh in channel B" scenario. The first
post-#1472 live run immediately broke the invariant and was not detected until a user
symptom surfaced.

### Missing: Write-path assertion

No assertion enforced G1 after writes. A `debug_assert!` in `write_session_state` after
the global write would have flagged violations in dev builds before they shipped.

### Missing: Schema version on snapshot

The snapshot had `schemaVersion` added in v2, but version 3 (inline `sessionId`) was not
designed because the session fix (#1479) was implemented as a separate registry write
rather than a co-located snapshot field. This creates two sources of truth that can drift.

---

## Implementation Checklist

### Landed (this session + previous PRs)

- [x] `normalize_snapshot_for_global` — strips `sourceBlockId` on global write (#1472)
- [x] `heal_global_snapshot_source_block_ids` — startup heal (#1472)
- [x] `backfill_session_ids` — startup session_id backfill (#1479)
- [x] `useHistoryPagination` hwm re-derive — fix stale hwm for agent-anchored restore (#1486)
- [x] `read_session_state` global-first — fix per-channel shadowing global (this session)

### Required (next PR)

- [ ] Write-path `debug_assert!` enforcing G1 after global write
- [ ] Snapshot schemaVersion 3: inline `sessionId` field
- [ ] `migrate_snapshots_v2_to_v3` startup migration
- [ ] `write_session_state` captures `sessionId` from in-memory session context
- [ ] `useHistoryPagination` prefers snapshot `sessionId` over registry for `--resume`

### Recommended (follow-up)

- [ ] e2e test: `run(channelA) → open(channelB) → assert full history + correct session_id`
- [ ] e2e test: `delete channels/ → open any agent → assert nothing lost`
- [ ] Per-channel write opt-out flag (for when the per-channel cache causes confusion)
- [ ] Agent export/import: serialize `shared/agents/<agentId>` as a portable `.agentx` bundle
- [ ] Cross-machine sync: `shared/agents/` as the sync target (iCloud / OneDrive friendly)

---

## Non-Goals

- **Pane layout portability:** Workspace pane arrangements are per-channel by design.
  They reference local blocks and local window geometry. Not in scope.
- **Memory file portability:** `db_memory_bundles` is per-channel. Globalization is a
  separate workstream.
- **Provider session sync across machines:** `session_id` enables resume on the same
  machine. Cross-machine provider session continuity requires the AI provider's API to
  support it (out of scope).

---

## References

- `agentmux-srv/src/backend/agent_session.rs` — write/read state, normalize, heal
- `agentmux-srv/src/backend/session_backfill.rs` — startup session_id backfill
- `agentmux-srv/src/server/app_api.rs:1066` — `global_output_source` (empty-local guard)
- `frontend/app/view/agent/hooks/useHistoryPagination.ts` — v2SameBlock, hwm re-derive
- `frontend/app/view/agent/agent-view.tsx:384` — `writeSnapshotNow`
- `docs/retro/retro-naki-history-recovery-all-attempts-2026-06-16.md` — incident history
- `docs/architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md` — data layout
