# SPEC: Agent-Pane Reducer State Persistence

**Status:** Draft / implementing
**Date:** 2026-05-15
**Author:** AgentA

---

## 1. Problem

When AgentMux closes and reopens, the agent-pane shows a conversation that's **visibly broken**: incomplete tables, truncated code blocks, orphaned tool-call results, missing middle turns.

**Root cause** (validated against the codebase):

1. The agent-pane reducer (`frontend/app/store/agent-document/reducer.ts`) is **in-memory only** — fresh on every pane mount.
2. On reopen, `useHistoryPagination` reads the **trailing 200 lines** of the FileStore NDJSON `output` file (`PAGE_SIZE = 200`, hard-coded) and replays them through `ClaudeCodeStreamParser` into `HistoryLoaded`.
3. For long conversations (thousands of events), 200 lines is a fraction. Anything BEFORE the window is gone. Anything CROSSING the boundary (a markdown table that started at line 50, a tool-call whose `tool_use_start` is at line 30 but `tool_use_result` at line 220) renders as a broken / orphaned fragment.
4. `loadOlder()` exists but **no UI wires it up** — users can't pull in more.

## 2. Goals

- **G1** A reopened pane shows the **complete** prior conversation, identical to pre-close.
- **G2** Scroll position is preserved (or sensibly defaulted: stick-to-bottom on long sessions, anchor-to-node when user had scrolled).
- **G3** Works with the existing TanStack Solid Virtual hybrid (virtualized head + 50-node streaming buffer).
- **G4** No regression for short conversations or fresh agents.
- **G5** Crash-safe: a hard kill of AgentMux should lose at most ~30 seconds of state.

## 3. Non-goals

- Persisting the agent-CLI's own internal session memory (it owns its own `~/.claude/projects/...` files).
- Cross-instance sync (one disk = one source of truth).
- Encrypting the snapshot (it's in the user's data dir, same trust boundary as the NDJSON).

## 4. Architecture

### 4.1 Storage: FileStore sidecar JSON

Each block already has a FileStore "output" file (NDJSON, one event per line). Add a sibling:

```
~/.agentmux/data/blocks/<blockid>/output            # existing NDJSON (every event)
~/.agentmux/data/blocks/<blockid>/output.state.json # NEW — reducer snapshot
```

**Why FileStore sidecar, not block-meta:**

- block-meta is a row in SQLite — fine for ~kilobyte values but bad for the 3–7 MB JSON a 10k-node session produces.
- FileStore is file-backed; reads stream from disk; writes atomic via temp+rename.
- The two files stay co-located; orphan cleanup naturally co-deletes them.
- The existing `BlockfileLineCountCommand` / `BlockfileReadRangeCommand` RPCs are the proximate model — add two parallels for the state file.

### 4.2 Snapshot schema (v1)

```typescript
interface AgentPaneStateSnapshot {
  /** Snapshot schema version. Bump on any breaking change. */
  schemaVersion: 1;
  /** ISO 8601, for debug + age display. */
  savedAt: string;
  /**
   * NDJSON line count at the time of the snapshot. On restore, the live
   * stream resumes reading lines >= highWaterMark. Without this, restored
   * state + new live events would either gap or double-emit.
   */
  highWaterMark: number;
  /** Full reducer state — see frontend/app/store/agent-document/types.ts. */
  nodes: DocumentNode[];
  /** Sticky-scroll flag from AgentViewState. */
  stickToBottom: boolean;
  /** Scroll anchor, or null if user was at bottom. */
  headAnchor: { nodeId: string; offsetPx: number } | null;
  /** Optional: collapsedNodes / pinnedNodes (already persisted via block meta — keep both during transition; deprecate meta later). */
  collapsedNodeIds?: string[];
  pinnedNodeIds?: string[];
}
```

### 4.3 Write triggers

1. **Eager: on reducer `Disposed`.** Best fidelity — final state. Written synchronously before pane teardown completes.
2. **Debounced: every 30 s during active streaming.** Crash-safety floor. Fires only if the document changed since last save. Skipped during pure scroll or pin/collapse (those write via existing block-meta path).
3. **On `TurnEnd`.** Cheap snapshot at a natural quiet point.

Writes are atomic: write to `output.state.json.tmp`, fsync, rename. A torn write thus surfaces as "no snapshot exists" → fall back to NDJSON replay path.

### 4.4 Read flow (replaces current init)

```
useHistoryPagination.onMount:
  1. Try BlockfileReadStateCommand → snapshot?
     - YES, schemaVersion matches:
         - Dispatch HistoryRestored { nodes, stickToBottom, headAnchor }
         - Set replayCursor = snapshot.highWaterMark
     - NO (missing, mismatched version, parse error):
         - Fall back to current PAGE_SIZE=200 trailing-line replay
         - Set replayCursor = totalLines - 200
  2. Restore scroll via existing restoreScrollFromAnchor(anchor, ...).
  3. Live-stream subscription starts reading lines >= replayCursor.
```

### 4.5 Reducer additions

```typescript
// agent-document/reducer.ts
| { type: "HistoryRestored";
    nodes: DocumentNode[];
    /** preserved for the view layer; doesn't change reducer behavior */
    fromSnapshot: true;
  }
```

`HistoryRestored` is a **full replace** of `nodes[]` (vs `HistoryLoaded` which prepends a partial chunk). The `fromSnapshot: true` discriminator lets the view layer distinguish snapshot restore from partial pagination and skip the "Loading older messages" affordance.

### 4.6 Backend RPCs

```rust
// agentmux-srv/src/server/blockfile_handlers.rs
BlockfileReadStateCommand { blockId } -> Option<String> // raw JSON
BlockfileWriteStateCommand { blockId, json: String } -> ()
```

Both are thin wrappers over a new `FileStore::read_sidecar(name)` / `write_sidecar(name, content)` pair.

## 5. Compatibility with virtualization

Already validated against the codebase:

- TanStack Solid Virtual's `createVirtualizer()` accepts the full `nodes[]` array immediately and renders only what's visible.
- `measureElement()` callbacks fire on first paint of each recycled row — heights settle without stutter.
- The 50-node streaming-buffer (`<Index>` not `<For>`) reads from the tail of the same `nodes[]` — restore works there too.
- `headAnchor` math (`restoreScrollFromAnchor` in `frontend/app/view/agent/anchor.ts`) is pure and unchanged.

**No virtualization changes required.**

## 6. Backwards compatibility

- Existing blocks have no `output.state.json` → fall back to the current 200-line NDJSON replay (no regression, no improvement).
- Future opens write a snapshot on close → next open restores fully.
- One-time "warm-up": on first open of an existing long conversation post-rollout, the user still sees the broken view. Closing + reopening once writes the snapshot from whatever the pane successfully rebuilt → second reopen is fine. **Or** add a one-shot "rebuild from full NDJSON" path (Phase 2) that streams the whole `output` file through the parser on first open if no snapshot exists. Recommend: ship without Phase 2; let it self-heal on next close.

## 7. Edge cases

| Case | Handling |
|---|---|
| `output.state.json` corrupt / partial | JSON parse fails → fall back to NDJSON replay. Atomic rename prevents most corruption. |
| `schemaVersion` mismatch | Fall back to NDJSON replay. Migration script optional. |
| `nodes[]` references a node id not in any live event | Render as-is; reducer is permissive. |
| Snapshot is much older than the NDJSON | Live-stream subscription catches up via `replayCursor`. |
| User opened a continued agent (different block id) | Each block has its own sidecar; no cross-talk. |
| Snapshot > 10 MB | Soft warn in tracing; don't refuse. Compression deferred to a future spec. |
| Multiple instances of the same block (Phase 3) | Latest writer wins. Phase 3 isn't shipping with this. |

## 8. Phased rollout

| Phase | Scope | LOC |
|---|---|---|
| **0** | Reducer: add `HistoryRestored` command + tests | ~50 |
| **1** | Backend: `Blockfile{Read,Write}StateCommand` + FileStore sidecar API | ~80 Rust |
| **2** | Frontend: write on Disposed, read on mount, fallback to current path | ~100 |
| **3** | Debounced 30 s save + write-on-TurnEnd | ~30 |
| **4** | (Optional) one-shot full NDJSON rebuild on missing-snapshot detection | ~60 |

Phases 0–3 ship together (one PR). Phase 4 deferred unless empirical demand.

## 9. Risk

- **Storage growth.** A user with 100 long-running agents would accumulate ~100 × 5 MB = 500 MB of state files. Acceptable; existing NDJSON files are already MB-scale.
- **Write IO during streaming.** Debounced 30 s + on-TurnEnd is bounded. Worst case: 1 write per 30 s of active streaming = trivial.
- **Schema drift.** If `DocumentNode` evolves and we forget to bump `schemaVersion`, old snapshots restore with the old shape and downstream rendering may break in subtle ways. Mitigation: enforce a `schemaVersion`-vs-current-typescript-types check in the reducer's `HistoryRestored` handler; on mismatch, fall back to NDJSON.

## 10. Open questions

- **Q1** Should `output.state.json` be gzip-compressed at rest? Saves 60–75% on disk. Adds a Rust gzip dep on the backend side. Decision: NOT in v1. Ship plain JSON, measure, compress later if needed.
- **Q2** Should the snapshot be **incremental** (event-sourced log + periodic compaction) rather than a full replace? More resilient but more complex. Decision: full-replace for v1; revisit if write IO becomes a problem.
- **Q3** Should `BlockfileWriteStateCommand` be exposed to the agent CLI itself (so agents can persist their own view metadata)? Probably not — that's a different feature surface. Out of scope.

---

🤖 Authored by AgentA. Implementation rides in the same PR — file is committed alongside the code change (per `feedback_no_doc_only_prs.md`).
