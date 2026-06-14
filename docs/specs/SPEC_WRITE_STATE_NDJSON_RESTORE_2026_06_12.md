# SPEC: write_state Option 4 — NDJSON-Reconstructed Snapshot (Schema v2)

**Status:** Draft  
**Date:** 2026-06-12  
**Author:** AgentA  
**Parent spec:** `SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md`  
**Motivation:** `docs/analysis/ANALYSIS_RENDERER_CRASH_WRITE_STATE_OOM_2026_06_12.md`

---

## 1. Problem

The schema-v1 snapshot (`output.state.json`) stores the entire `DocumentNode[]` array.
For a long session this reaches 7–10 MB. `writeSnapshotNow` serialises it via
`JSON.stringify` on the main render thread every 30 s, allocating a full-size string
and a same-size `Blob` regardless of whether the WebSocket cap discards the write.
This 126 ms+ long-task spikes memory and eventually triggers a renderer OOM crash (see
analysis doc above).

The underlying invariant is already correct: every event is already durably written
to the NDJSON `output` blockfile by `AgentSessionAppendOutputCommand`. The frontend
has everything it needs to reconstruct `DocumentNode[]` from that log; it does so
today on the "slow path" (200-line ring-buffer replay). Option 4 exploits this: stop
sending nodes in the snapshot; send only the UI-overlay state that cannot be
reconstructed from NDJSON.

---

## 2. Design principle: unlimited history, disk is the only constraint

A conversation session may grow to any size — 23 K lines today, 1 billion lines in
the future.  AgentMux imposes **no artificial cap on session length**.  The only
constraint is available disk space.  When disk space runs low, the app warns the
user; it does not silently discard history or refuse to write.

`RESTORE_WINDOW_LINES` (§6.3) is a **render viewport**, not a storage limit.  It
controls how much history is loaded into the renderer on first open — analogous to
a terminal emulator's scrollback buffer.  Everything outside the window is still on
disk and reachable via load-older.  The user never loses history to this constant.

## 3. Goals

- **G1** Snapshot payload drops below ~5 KB for any session length. No more OOM.
- **G2** Full conversation accessible regardless of length — via load-older if
  outside the initial render viewport.
- **G3** User-controlled UI state (collapsed nodes, pinned nodes, scroll position,
  filter toggles) is preserved across close/reopen.
- **G4** Backward-compatible: v1 snapshots continue to restore via the existing code
  path until they age out.
- **G5** No new sidecar parsing logic — reconstruction runs the existing
  `parseHistoryLines` pipeline in the frontend.
- **G6** Disk-space warning surfaced in the agent pane before the disk is full,
  with a clear action (archive / free space). No silent data loss.

### Non-goals

- Making restoration faster than the v1 fast path. (NDJSON replay is inherently
  slower than a pre-serialised `nodes[]` load. This is an acceptable trade.)
- Persisting the agent CLI's own session memory.
- Cross-instance sync.
- Limiting how long a conversation can be.

---

## 3. State inventory: what lives where

### 3.1 Fully reconstructable from NDJSON

These fields are produced deterministically by replaying `output` lines through
`parseHistoryLines(lines, outputFormat)`:

| Field | How reconstructed |
|---|---|
| `DocumentNode[]` (all kinds) | Full `output` log, lines `[0, highWaterMark)` |
| Node `id`, `type`, `status`, `result` | Carried in NDJSON events |
| Tool params, markdown content, thinking | Carried in NDJSON events |
| `SectionNode.isStartup` | Carried in NDJSON events |

### 3.2 NOT reconstructable — must persist in snapshot overlay

These are UI state produced by user interaction, absent from the event stream:

| Field | Type | Default |
|---|---|---|
| `collapsedNodes` | `string[]` (serialised `Set<string>`) | `[]` |
| `pinnedNodes` | `string[]` (serialised `Set<string>`) | `[]` |
| `scrollPosition` | `number` (px from top) | `0` |
| `filter.showThinking` | `boolean` | `false` |
| `filter.showSuccessfulTools` | `boolean` | `true` |
| `filter.showFailedTools` | `boolean` | `true` |
| `filter.showIncoming` | `boolean` | `true` |
| `filter.showOutgoing` | `boolean` | `true` |
| `paneState.detailsOpen` | `boolean` | `false` |
| `historyOffset` | `number` | `0` |

`selectedNode` is transient cursor state and is deliberately NOT persisted.

---

## 4. Schema v2

```typescript
interface AgentPaneStateSnapshotV2 {
  schemaVersion: 2;
  savedAt: string;          // ISO 8601

  /**
   * NDJSON line count at save time.  On restore, fetch lines [0, highWaterMark)
   * from the `output` blockfile and replay through parseHistoryLines.  After
   * restore, the live-stream subscription resumes from this line index so
   * background events written while the pane was closed are replayed exactly
   * once.
   */
  highWaterMark: number;

  /**
   * NDJSON line index where the "load older" cursor should start.  Unchanged
   * from v1.  On mount after restore, loadOlder() fetches
   * [historyOffset - PAGE_SIZE, historyOffset) and prepends the parsed nodes.
   */
  historyOffset: number;

  /** Serialised DocumentState overlay — applied on top of reconstructed nodes. */
  documentState: {
    collapsedNodeIds: string[];
    pinnedNodeIds: string[];
    scrollPosition: number;
    filter: {
      showThinking: boolean;
      showSuccessfulTools: boolean;
      showFailedTools: boolean;
      showIncoming: boolean;
      showOutgoing: boolean;
    };
  };

  /** Serialised AgentPaneState overlay. */
  paneState: {
    detailsOpen: boolean;
  };
}
```

Persisted as `output.state.json` in the `agent:<definitionId>:current` zone — same
file, same location, same sidecar RPC (`agent:session:write_state`) as v1.  Only
the JSON content changes.

---

## 5. Write path changes (frontend)

**File:** `frontend/app/view/agent/agent-view.tsx`

### 5.1 Replace `writeSnapshotNow`

```typescript
const writeSnapshotNow = () => {
    const nodes = getDocument();
    if (!nodes) return;

    // Capture UI overlay state before the async chain so we get the
    // values at the moment of the save trigger, not after a potential
    // 3 s RPC round-trip.
    const [docState] = agentAtoms().documentStateAtom;
    const [detailsOpen] = agentAtoms().detailsOpenAtom;
    const capturedOffset = history.historyOffset();
    const capturedDocState = docState();
    const capturedDetailsOpen = detailsOpen();

    inFlightSnapshot = inFlightSnapshot.then(async () => {
        let highWaterMark = 0;
        try {
            const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                block_id: model.blockId,
                filename: "output",
            }, { timeout: 3000 });
            highWaterMark = countResp?.count ?? 0;
        } catch {
            // Soft fail.
        }

        const snapshot: AgentPaneStateSnapshotV2 = {
            schemaVersion: 2,
            savedAt: new Date().toISOString(),
            highWaterMark,
            historyOffset: capturedOffset,
            documentState: {
                collapsedNodeIds: [...(capturedDocState?.collapsedNodes ?? [])],
                pinnedNodeIds:    [...(capturedDocState?.pinnedNodes    ?? [])],
                scrollPosition:   capturedDocState?.scrollPosition ?? 0,
                filter:           capturedDocState?.filter ?? DEFAULT_FILTER_STATE,
            },
            paneState: {
                detailsOpen: capturedDetailsOpen,
            },
        };

        await RpcApi.AgentSessionWriteStateCommand(TabRpcClient, {
            definition_id: agentId,
            content: JSON.stringify(snapshot),
        }, { timeout: 10000 });
    }).catch((e) => {
        log("history", `snapshot write failed: ${e?.message ?? e}`, "warn");
    });
};
```

`JSON.stringify` of this object is under 1 KB for any session. The OOM trigger is
gone regardless of conversation length.

### 5.2 Remove node-count log noise

The v1 restore path logged `restored N nodes from snapshot`. Update the message to
reflect that nodes come from NDJSON replay (see §6.3).

---

## 6. Read path changes (frontend)

**File:** `frontend/app/view/agent/hooks/useHistoryPagination.ts`

### 6.1 Schema version constant

```typescript
export const SNAPSHOT_SCHEMA_VERSION_V1 = 1;
export const SNAPSHOT_SCHEMA_VERSION_V2 = 2;
export const SNAPSHOT_SCHEMA_VERSION = SNAPSHOT_SCHEMA_VERSION_V2; // current write version
```

### 6.2 Restore dispatch

Introduce a new reducer action to apply the overlay state after NDJSON nodes are set:

```typescript
// agent-document/reducer.ts
| {
    type: "HistoryRestored";
    fromSnapshot: true;
    nodes: DocumentNode[];
    // v2 additions (optional — absent when restoring a v1 snapshot)
    documentStateOverlay?: {
        collapsedNodes: Set<string>;
        pinnedNodes: Set<string>;
        scrollPosition: number;
        filter: FilterState;
    };
    paneStateOverlay?: {
        detailsOpen: boolean;
    };
  }
```

The reducer applies `documentStateOverlay` to `state.documentState` and
`paneStateOverlay` to the pane slice if present.  When absent (v1 restore), it
behaves exactly as before.

### 6.3 Fast path — v2 snapshot

```typescript
// Render viewport: how many NDJSON lines to load on first open.
// This is NOT a storage cap — history grows without bound until disk
// runs out (§2, §14).  Lines outside the window are on disk and
// accessible via load-older.  5 000 lines ≈ the last few hundred
// turns, which is the practical "recently visible" window.
// See §13 for the memory/seek analysis behind this number.
const RESTORE_WINDOW_LINES = 5_000;

if (snapshot.schemaVersion === SNAPSHOT_SCHEMA_VERSION_V2) {
    // Step 1: fetch at most RESTORE_WINDOW_LINES NDJSON lines ending at
    // highWaterMark.  This bounds the response size regardless of how old
    // the session is.
    const hwm = snapshot.highWaterMark ?? 0;
    const windowStart = Math.max(0, hwm - RESTORE_WINDOW_LINES);
    let nodes: DocumentNode[] = [];
    if (hwm > 0) {
        const rangeResp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
            block_id: opts.blockId,
            filename: "output",
            offset: windowStart,
            limit: hwm - windowStart,
        }, { timeout: 30_000 });
        if (!mounted) return;
        nodes = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat());
    }

    // Step 2: reconstruct overlay state.
    const ds = snapshot.documentState;
    const documentStateOverlay = ds ? {
        collapsedNodes: new Set<string>(ds.collapsedNodeIds ?? []),
        pinnedNodes:    new Set<string>(ds.pinnedNodeIds    ?? []),
        scrollPosition: ds.scrollPosition ?? 0,
        filter:         ds.filter ?? DEFAULT_FILTER_STATE,
    } : undefined;
    const paneStateOverlay = snapshot.paneState ?? undefined;

    // Step 3: dispatch.
    batch(() => opts.model.dispatchDoc({
        type: "HistoryRestored",
        fromSnapshot: true,
        nodes,
        documentStateOverlay,
        paneStateOverlay,
    }));

    // historyOffset is the load-older cursor — the start of the restore
    // window, not the value stored in the snapshot (which was the cursor
    // position at save time, potentially further back in a very long
    // session).  Using windowStart ensures load-older pages correctly
    // from just before the restore window.
    setHistoryOffset(windowStart);
    setHistoryTotal(hwm);
    opts.onContinuationModts?.(stateResp.modts ?? 0);
    opts.log(
        "history",
        `v2 restore: ${nodes.length} nodes from lines [${windowStart}, ${hwm})` +
        (windowStart > 0 ? ` (${windowStart} older lines available via load-older)` : "") +
        (ds?.collapsedNodeIds?.length ? `, ${ds.collapsedNodeIds.length} collapsed` : "") +
        (ds?.pinnedNodeIds?.length    ? `, ${ds.pinnedNodeIds.length} pinned`       : ""),
    );
    opts.model.dispatchPane({ type: "InitReady", at: Date.now() });
    opts.onHistoryReady?.();
    return;
}
```

### 6.4 Legacy v1 snapshot

The existing `schemaVersion === 1 && Array.isArray(snapshot.nodes)` branch is
unchanged.  v1 snapshots continue to restore nodes directly until they are
overwritten by a v2 write or age out.

### 6.5 Fallback chain (unchanged)

```
v2 snapshot → full NDJSON replay + overlay
v1 snapshot → direct node restore (existing path)
no snapshot → trailing-200-line NDJSON replay (existing path)
```

---

## 7. Sidecar changes

**None required for the core fix.** The sidecar writes whatever `content` string it
receives from `agent:session:write_state` to `output.state.json`.  A v2 payload is
smaller than a v1 payload; all existing validation (atomic write via temp+rename)
continues to apply.

### 7.1 `BlockfileReadRangeCommand` is unbounded — window matters

`CommandBlockfileReadRangeData` has `offset: u64, limit: u64` with no server-side
cap.  Passing `offset: 0, limit: 10_000_000` returns 10 M lines as a `Vec<String>`
in one WebSocket message — potentially gigabytes.  The `RESTORE_WINDOW_LINES = 5000`
bound in §6.3 is therefore essential; the sidecar will not protect against an
oversized request.

A 5 000-line response is ~600 KB–3 MB of NDJSON text — comfortably within one
WebSocket message.  `parseHistoryLines` over 5 000 lines produces at most a few
hundred `DocumentNode[]` items.

---

## 8. Migration

| Snapshot found | schemaVersion | Has `nodes` | Action |
|---|---|---|---|
| None | — | — | Slow path (trailing 200-line replay) |
| v1 | 1 | ✓ | v1 fast path (direct node restore, no change) |
| v2 | 2 | ✗ | v2 fast path (NDJSON replay + overlay) |
| Unknown version | other | any | Fall through to slow path (logged warn) |

v1 snapshots expire naturally: the next 30 s dirty-flag tick after the first v2
write overwrites `output.state.json` with a v2 payload.  No explicit migration step
is needed.

---

## 9. DocumentState wiring

`documentStateAtom` lives in `agentAtoms()` (created by `createAgentAtoms` in
`state.ts:119`).  `agent-view.tsx` already holds a reference.  The capture in
§5.1 reads it synchronously before the async RPC chain, so the snapshot reflects
the UI state at trigger time.

On restore (§6.3), the overlay is delivered to the reducer via `HistoryRestored`.
The reducer applies it to `state.documentState`; the `AgentDocumentView` component
re-reads from the atom on the next reactive tick.

`collapsedNodes` and `pinnedNodes` are `Set<string>` keyed by node ID.  These IDs
are stable: `parseHistoryLines` produces the same IDs for the same NDJSON input
because IDs are embedded in the events (not derived from array index).  The overlay
therefore correctly re-opens/closes the same nodes that were open/closed at save
time.

---

## 10. Performance profile

| Operation | v1 | v2 |
|---|---|---|
| Snapshot serialise | `JSON.stringify(N nodes)` — 10–126 ms, 7–10 MB alloc | `JSON.stringify(overlay)` — <1 ms, <1 KB |
| Snapshot write | Same RPC, slower for large payloads | Same RPC, ~1 KB payload |
| Restore — network | 0 extra RPCs (nodes embedded) | 1 `BlockfileReadRangeCommand` (≤ 5000 lines) |
| Restore — parse | 0 (nodes pre-parsed) | `parseHistoryLines` over ≤ 5000 lines (~100–300 ms) |
| Restore — wall clock | ~50–100 ms (WebSocket deserialise) | ~200–500 ms for a full 5000-line window |
| Max response size | Grows unbounded (OOM) | Always ≤ ~3 MB regardless of session length |

The restore latency increase is acceptable: the UI displays the `InitPending` loading
state during this window.  The OOM elimination far outweighs the added restore time.

Critically, **restore time is O(1) with respect to session length** because the
window is fixed at `RESTORE_WINDOW_LINES`.  A 10 M-line session restores in the same
time as a 5 000-line session.

---

## 11. Test plan

### Unit

- `writeSnapshotNow` sends a payload with `schemaVersion: 2` and no `nodes` key.
- Serialised payload for a 10k-node document is < 5 KB.
- `HistoryRestored` with `documentStateOverlay` applies `collapsedNodes` /
  `pinnedNodes` / `filter` to reducer state.

### Integration (in-app)

1. Open a long session (> 500 NDJSON lines).
2. Collapse two agent-message nodes; pin one tool node; scroll to mid-session.
3. Close and reopen the pane.
4. Verify: all conversation nodes present; collapsed nodes are collapsed; pinned node
   is pinned; scroll position approximately restored.
5. Verify no "ws message too large" log lines during the test.

### Regression

- Short sessions (< 50 nodes): restore still works; no extra RPC latency visible.
- v1 snapshot on disk: falls through to v1 fast path; no regression.
- No snapshot: falls through to slow path; no regression.

---

## 12. Open questions

1. **`BlockfileReadRangeCommand` line cap.** Does the sidecar clamp `limit`?  If
   yes, set the v2 restore loop's page size accordingly and document the cap.

2. **Background-agent gap fill.** If the agent ran while the pane was closed, NDJSON
   lines may exist in `[highWaterMark, currentTotal)`.  The live-stream subscription
   will deliver these from subscribe-time forward.  A gap (events that landed after
   save but before subscribe) is the same v1 limitation.  Phase 4 of the parent spec
   addresses this; no change here.

3. **`scrollPosition` restore accuracy.** The virtual list scroll model maps pixel
   positions to node indices; a node that renders differently after NDJSON replay vs.
   snapshot may shift the target.  A best-effort `scrollPosition` restore (snap to
   nearest node boundary) is sufficient.  Exact pixel matching is not a goal.

4. **`detailsOpen` wiring.** Confirm `detailsOpenAtom` is accessible from
   `agent-view.tsx` (it is on `agentAtoms()`) and that `HistoryRestored` can set it.
   If the pane state atom lives outside the reducer, apply it in the `batch()` call
   via a separate `dispatchPane` action rather than embedding it in `HistoryRestored`.

---

## 13. Render viewport analysis (why 5 000 lines)

### 13.1 Renderer memory limits that motivate the viewport size

`BlockfileReadRangeCommand` has no server-side line cap (`rpc_types.rs:1020–1025`).
For a session with N NDJSON lines:

| N | Approx file size | `Vec<String>` response | `DocumentNode[]` after parse | V8 heap impact |
|---|---|---|---|---|
| 5 000 | ~600 KB | ~3 MB | ~200–800 nodes | < 50 MB |
| 50 000 | ~6 MB | ~30 MB | ~2 000–8 000 nodes | ~200 MB |
| 500 000 | ~60 MB | ~300 MB | ~20 000–80 000 nodes | ~2 GB — OOM |
| 10 000 000 | ~1.2 GB | ~6 GB | out of memory | crash |

The `RESTORE_WINDOW_LINES = 5_000` constant keeps all four columns in the first row
regardless of actual session length.

### 13.2 How long does it take to reach 10 M lines?

Each agent turn (user message → assistant response with tools) produces roughly
500–2 000 NDJSON lines (streaming deltas, tool events, results).  A heavy usage
day like the crash day (2 long sessions, ~8 hours) produced an estimated 50 000–
200 000 lines.  Reaching 10 M lines would require weeks to months of continuous
heavy use.

The more immediate concern is reaching the ~50 000 threshold (a few weeks for power
users) where an unbounded restore would produce 2 GB+ of V8 allocations.  The
bounded window addresses this.

### 13.3 Disk growth is intentional — not bounded here

The `output` blockfile grows without bound by design (§2).  At 10 M lines × 120
bytes that is ~1.2 GB per agent session.  This is fine — disk is cheap and the data
is valuable conversation history.  The only response to disk growth is the low-disk
warning (§14), not a silent cap.

The existing `agent:session:archive` RPC (`rpc_types.rs:2059–2072`) is a user-
initiated action to snapshot-and-clear the current zone.  It is not called
automatically; the user opts in.  A future "Start new session" or "Archive
conversation" button in the pane UI would wire up this RPC.

### 13.4 Seek cost — why the byte-offset index is required for large sessions

**The invariant we want:** opening a pane backed by a 100 GB session history feels
instant.  During live use this is already true — live events arrive over WebSocket
and never touch the NDJSON file.  The risk is the **initial open**: the sidecar must
seek to line `(totalLines - RESTORE_WINDOW_LINES)` to serve the restore request.

With the current flat-file implementation in `readutil.rs`, that seek scans from
byte 0 to find the target line's byte offset — O(file size).  Results:

| File size | Seek time (approx) |
|---|---|
| 18 MB (today's Mopeo session) | < 10 ms ✓ |
| 600 MB (~500 K lines) | ~300 ms — noticeable |
| 6 GB (~5 M lines) | ~3 s — slow |
| 100 GB (~80 M lines) | ~50 s — unusable |

**The fix: a persistent byte-offset index (`output.idx`).**

On every `append_output` call, the sidecar appends the current end-of-file byte
offset as a little-endian u64 to a companion file `output.idx` in the same zone.
After N appends, `output.idx` is exactly `N × 8` bytes.  To seek to line K:

```
byte_offset = read_u64(output.idx, offset = K × 8)   // one 8-byte read, O(1)
seek(output, byte_offset)                              // OS seek, O(1)
```

With the index, **open time is O(1) regardless of session size**.  A 100 GB session
opens in the same time as an 18 MB session.

`readutil.rs::read_last_n_line_offsets` already builds a transient in-memory version
of this index for tail reads.  Persisting it is a small additional write per
`append_output` call.

**This index is a prerequisite for the "unlimited history" design principle (§2).**
Without it, sessions beyond ~50 MB exhibit a perceptible open delay, and sessions
beyond ~1 GB are unusable.  It should be implemented in the same PR as the v2
snapshot or shortly after, before users encounter large sessions in the wild.

The index file itself scales as `N × 8` bytes.  At 1 billion lines it is 8 GB —
large but seekable in O(1) (fixed-width records, no scan needed).  If the index
itself becomes unwieldy, segment it alongside the data file.

---

## 14. Disk-space warning

### 14.1 Principle

The only reason to surface a warning about session size is **impending disk
exhaustion**.  AgentMux must not cap history, compress it, or refuse writes.  It
must warn the user in time for them to act (free space, archive a session, or buy
more disk) before writes start failing silently.

### 14.2 Warning levels

| Level | Condition | UI surface |
|---|---|---|
| **Advisory** | Free disk < 1 GB | Subtle banner at top of agent pane: "Disk space is running low — X MB free" |
| **Warning** | Free disk < 500 MB | Orange banner + badge on the pane tab |
| **Critical** | Free disk < 200 MB | Modal dialog blocking new agent turns: "Almost out of disk space. AgentMux may stop saving conversation history. Free space to continue." |

Thresholds are conservative intentionally: SQLite WAL files, CEF cache, and other
AgentMux data also consume space.  The advisory fires early to leave headroom.

### 14.3 How disk space is measured

The sidecar has OS-level access to `statvfs` (Unix) / `GetDiskFreeSpaceEx`
(Windows) against the data dir mount point (`~/.agentmux/`).  A new
`system:disk_stats` RPC returns:

```typescript
interface DiskStats {
  data_dir_bytes_used: number;   // total bytes in ~/.agentmux/ channel data dir
  disk_free_bytes: number;       // free bytes on the containing volume
  disk_total_bytes: number;      // total capacity of the volume
}
```

The frontend polls this every 60 s (not on every turn — cheap enough).  The 60 s
interval means the warning appears within a minute of crossing a threshold.

### 14.4 Per-session size in the agent pane

When `data_dir_bytes_used` / `session_bytes` is available, the agent pane settings
panel (cog → Info tab) shows:

```
Conversation history: 18.2 MB (23 314 lines)
Total AgentMux data:  46 MB
Free disk:           234 GB
```

This is informational, not actionable — no "delete" button here.  The archive
action ("Start new session") is a separate surface.

### 14.5 Implementation sketch

**Sidecar** (`agentmux-srv`):
- Add `COMMAND_SYSTEM_DISK_STATS = "system:disk_stats"` RPC.
- Handler calls `statvfs` / `GetDiskFreeSpaceEx` on the data dir path, returns
  `DiskStats`.  No new dependencies; both syscalls are in `std`.

**Frontend** (`agent-view.tsx` or a shared hook):
- `useDiskStats()` hook: polls `system:disk_stats` every 60 s, returns the latest
  `DiskStats` signal.
- `DiskWarningBanner` component: reads `diskStats().disk_free_bytes`, renders the
  appropriate level (§14.2) or nothing.
- Inserted above the document list in `AgentDocumentView`, below the session digest
  banner.

### 14.6 What the critical modal says

```
Disk space critical
Your disk has less than 200 MB free. AgentMux conversation
history may stop saving.

• Free up disk space, then dismiss this warning.
• Or archive this session to release space used by its
  history (the conversation is preserved in the archive).

[Archive this session]  [Dismiss]
```

"Archive this session" calls `agent:session:archive` for the current `definition_id`
and closes the modal.  "Dismiss" closes without action; the modal re-appears after
5 minutes if the condition persists.
