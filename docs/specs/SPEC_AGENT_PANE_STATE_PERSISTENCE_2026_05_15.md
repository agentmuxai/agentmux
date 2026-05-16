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
   *
   * v1 status: WRITTEN but NOT YET ACTED ON. The live-stream subscription
   * currently only delivers events from subscribe-time forward (no gap
   * replay). The "background agent ran while pane closed" gap is a known
   * v1 limitation. Phase 4 (deferred) will read NDJSON lines from
   * `highWaterMark..total` on reopen and dispatch them through the
   * normal StreamFlush path before the live subscription starts.
   */
  highWaterMark: number;
  /**
   * NDJSON line index where the loaded slice begins. On restore, this
   * becomes the `loadOlder` cursor — calls fetch lines in
   * `[offset - PAGE_SIZE, offset)` and prepend the parsed nodes via
   * `HistoryLoaded` (which dedupes by id, so any overlap with the
   * snapshot is harmless).
   *
   * MUST be the actual NDJSON line index, not derived from
   * `nodes.length` — streaming produces many NDJSON lines per
   * `DocumentNode` (token deltas, partial tool events), so node count
   * is not a proxy for line count. Reagent P1 on PR #877 round 3.
   */
  historyOffset: number;
  /** Full reducer state — see frontend/app/store/agent-document/types.ts. */
  nodes: DocumentNode[];
  /**
   * Sticky-scroll flag from AgentViewState.
   *
   * v1 status: NOT YET PERSISTED. `AgentViewState` is created inside
   * `AgentDocumentView` (a child of `agent-view.tsx` where the snapshot
   * save lives). Lifting it requires either a callback-ref prop or
   * moving `createAgentViewState` up. Deferred to Phase 4.
   */
  stickToBottom?: boolean;
  /**
   * Scroll anchor, or null if user was at bottom.
   *
   * v1 status: NOT YET PERSISTED — same wiring concern as `stickToBottom`.
   * Deferred to Phase 4.
   */
  headAnchor?: { nodeId: string; offsetPx: number } | null;
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

`HistoryRestored` prepends snapshot nodes onto current state with id-dedup (same semantics as `HistoryLoaded`), and additionally flips `sessionPhase` directly to `"active"`. The prepend semantics — rather than a full replace — are necessary because `useAgentStream` subscribes to the live event subject in the same component mount; `StreamFlush` can land before the async `BlockfileReadStateCommand` resolves, and a full replace would wipe those live arrivals (codex P1 on round 4). On id collision the existing (live) node wins; the snapshot's stale copy is dropped. The `fromSnapshot: true` discriminator lets the view layer distinguish snapshot restore from partial pagination and skip the "Loading older messages" affordance.

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

## 11. Known follow-ups surfaced by this PR

### 11.1 Lifecycle / dispatch leak (cross-cutting)

The first smoke build of v0.33.899 produced an uncaught `dispatch for unregistered pane c0ae6c7 (cmd=StreamFlushObserved)` in `useAgentStream.flushPendingNodes`. Root cause is **not** specific to this PR — it's a latent class of bug where a `dispatchDoc` write cascades through reactive subscribers and synchronously disposes the pane *during* the dispatch, causing the next dispatch in the same callback to throw.

This PR is the likely **trigger** (the new `createEffect` on `documentAtom` is the only new subscriber to that atom in months), but the underlying lifecycle vulnerability has been present since the multi-store pattern landed in §4 of `frontend-reducer-conventions-2026-05-03.md`.

Full breakdown: `docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md`. Cascade-detection instrumentation has been installed in `agent-pane-state-store.ts` and `agent-document-store.ts`; the next reproduction will identify the exact projection setter that triggers the cascade.

**Followup PRs** (status):
- **PR #878** ships the cascade-detection instrumentation AND the soft-dispatch migration in a single bundle. Adds `dispatchIfRegistered` to both pane stores, migrates 22 async dispatch sites across `useAgentStream.ts`, `useAgentCommands.ts`, `useHistoryPagination.ts`, `agent-view.tsx` (onLoginSuccess callback), and fixes the unguarded RAF in `browser-model.reload()`. Backed by 5 new regression tests in `agent-pane-state-store.test.ts`.
- **PR-3** (architectural, pending): unify per-pane registration so both stores' slots are added/removed atomically. Pre-discussion in #707.

### 11.2 Phase 4 still deferred

The `highWaterMark` gap-replay (read NDJSON lines from `[snapshot.highWaterMark, currentTotal)` on reopen and dispatch through `StreamFlush`) remains v1-deferred per §4.2. Should ship after the lifecycle work above so its new dispatches can use the soft-variant from the start.

---

🤖 Authored by AgentA. Implementation rides in the same PR — file is committed alongside the code change (per `feedback_no_doc_only_prs.md`). §11 added 2026-05-15 after the v0.33.899 smoke-build crash surfaced the lifecycle issue documented in `docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md`.
