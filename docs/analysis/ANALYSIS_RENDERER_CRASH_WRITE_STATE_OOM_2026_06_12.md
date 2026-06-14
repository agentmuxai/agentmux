# Renderer Crash — agent:session:write_state OOM / Long-Task

**Date observed:** 2026-06-12  
**Log evidence:** `~/.agentmux/logs/agentmux-host-v0.44.1.log.2026-06-12`  
**Crash count:** 1 today (08:17:18 UTC); 4 yesterday (14:06–14:11 UTC, three of which say `detail:"Out of Memory"`)

---

## Observed Symptoms

```
08:17:16  [fe] ws message too large 7722686  (agent:session:write_state)
08:17:16  [perf] long-task 126.0ms
08:17:18  renderer process crashed  error_code=-36861  Crashpad_NotConnectedToHandler
```

Recovery dialog shown: "AgentMux hit a problem — renderer process crashed".

Session data was safe on disk. User needed to click "Reload window."

### Growth pattern today (293 oversized messages, 00:33–08:48 UTC)

```
~05:00 UTC  messages start at ~5.2 MB (just over the 5 MB MaxWebSocketSendSize limit)
~07:00 UTC  messages reach ~7 MB
~08:17 UTC  7.72 MB spike + 126 ms long-task → crash
~08:30+     messages continue at 7.7–9.0 MB after recovery
```

The same two agent sessions (`definition_id` prefixes `38634ce` and `6dc8f91`) drive all of these; both have been running across the full session and accumulating conversation state.

---

## Root Cause Analysis

### What write_state does

`writeSnapshotNow` in `agent-view.tsx:325` runs every 30 s (dirty-flag gated) and on pane close. It:

1. Captures `nodes = getDocument()` — the full in-memory SolidJS document array (every message, tool block, thinking block, diff, etc. across the entire conversation).
2. Calls `JSON.stringify({ schemaVersion, savedAt, highWaterMark, historyOffset, nodes })`.
3. Measures the resulting string with `new Blob([msg]).size` (`ws.ts:231`).
4. **If ≤ 5 MB:** sends over WebSocket to the sidecar, which writes `output.state.json` to disk.
5. **If > 5 MB:** logs "ws message too large" and drops the send. **The serialized string is already in memory at this point.**

### Why it crashes

The renderer crash is a **renderer-process OOM or hard-kill**, not a JavaScript exception. Three compounding factors:

**1. Full-document serialization grows without bound.**  
Every conversation turn appends nodes. A long session with many tool results, diffs, and large BashOutputViewer payloads can push `nodes` to several MB of raw JSON. There is no cap on what gets included in the snapshot.

**2. JSON.stringify + Blob happen on the main render thread.**  
Even when the message is too large and dropped, `JSON.stringify(snapshot)` already built a 7–10 MB string. `new Blob([msg])` built a 7–10 MB Blob. Both are allocated in the renderer's V8 heap. The 126 ms long-task is this serialization. Doing this every 30 s with an ever-growing document starves the render loop and spikes memory.

**3. Memory never comes down.**  
The `nodes` signal grows monotonically during a session. No trimming or offloading occurs. By the time the message reaches 7–8 MB, the renderer's resident set has been high for hours (peak_ws_mb 201.6 MB was recorded much earlier today). The spike from a large `JSON.stringify` call is enough to trigger the OOM path.

**The 5 MB WebSocket cap is not the fix.** It prevents data loss on the wire but does nothing to reduce the serialization cost that causes the crash. The renderer creates the full string regardless.

---

## Code Locations

| File | Line | Role |
|------|------|------|
| `frontend/app/view/agent/agent-view.tsx` | 325–380 | `writeSnapshotNow` + 30 s interval |
| `frontend/app/view/agent/agent-view.tsx` | 340–357 | `JSON.stringify(snapshot)` + `write_state` call |
| `frontend/app/store/ws.ts` | 12–13 | `WarnWebSocketSendSize = 1 MB`, `MaxWebSocketSendSize = 5 MB` |
| `frontend/app/store/ws.ts` | 230–239 | `sendMessage` — serializes, measures, drops if > 5 MB |
| `agentmux-srv/src/backend/rpc_types.rs` | 2033 | `CommandAgentSessionWriteStateData.content: String` |
| `agentmux-srv/src/server/agent_handlers.rs` | 2027 | Backend handler — writes `output.state.json` |

---

## Fix Options

### Option 1 — Cap nodes included in the snapshot (recommended, low risk)

Include only the last N nodes (e.g. 200) in the `write_state` snapshot. Older nodes are already durably persisted in the `output` NDJSON log (written incrementally by `AgentSessionAppendOutputCommand`). On session restore, replay old nodes from NDJSON up to `highWaterMark`, then overlay the snapshot's recent nodes.

**Pros:** Bounded snapshot size regardless of conversation length. Minimal behaviour change — restoration already uses `highWaterMark` to know where NDJSON replay ends. Safe to ship quickly.  
**Cons:** Requires `historyRestore` to handle the case where the snapshot covers only the tail; test coverage needed.

```typescript
// agent-view.tsx — before JSON.stringify
const MAX_SNAPSHOT_NODES = 200;
const snapshotNodes = nodes.length > MAX_SNAPSHOT_NODES
    ? nodes.slice(-MAX_SNAPSHOT_NODES)
    : nodes;
const snapshot = { schemaVersion, savedAt, highWaterMark, historyOffset, nodes: snapshotNodes };
```

### Option 2 — Measure size before serializing (quick defensive fix)

Estimate document size before committing to full serialization. An approximation (e.g. sum of `node.content?.length` across nodes) can bail out early before the expensive `JSON.stringify`.

**Pros:** Prevents the main-thread spike when the document is known to be large.  
**Cons:** Heuristic — doesn't actually reduce the snapshot size on disk, just skips the save. A large document is never snapshotted, so crash recovery would fall back to NDJSON replay (which might be slower).

### Option 3 — Offload serialization to a Web Worker

Run `JSON.stringify(snapshot)` in a `Worker` so it doesn't block the render thread. The 30 s interval posts a structured-clone of `nodes` to the worker, which serializes and posts back the string.

**Pros:** Eliminates the long-task / jank.  
**Cons:** Structured-clone of large `nodes` is itself O(n). Does not reduce memory pressure in the render process. Higher implementation complexity.

### Option 4 — Backend-reconstructed state (longer term)

The sidecar already has the full conversation via the incrementally-appended `output` NDJSON log. Instead of the frontend sending the full document on every save, it could send only the lightweight `AgentPaneState` reducer state (turn phase, scroll position, filters, etc. — a few KB). On restore, the sidecar reconstructs `nodes` from the NDJSON log and the reducer state is applied on top.

**Pros:** Eliminates the serialization problem at its root.  
**Cons:** Requires backend changes to reconstruct `DocumentNode[]` from NDJSON, and careful matching with frontend reducer replay. Significant scope.

---

## Recommended Short-Term Fix

**Ship Option 1** (node cap) as a quick patch. 200 nodes covers any realistic "recent context" need; older history is safely replayed from the NDJSON log. This immediately bounds the snapshot payload to roughly 1–2 MB and eliminates the OOM trigger.

Pair it with a log warning when nodes are capped so the behaviour is observable:

```typescript
if (nodes.length > MAX_SNAPSHOT_NODES) {
    log("history", `snapshot capped at ${MAX_SNAPSHOT_NODES}/${nodes.length} nodes (older nodes replay from NDJSON)`, "info");
}
```

**Longer term** (Option 4) is the right architecture — the frontend shouldn't be the source of truth for the full conversation; the append-only NDJSON log already is.

---

## Recurrence Risk

Without a fix, this crash will recur on any session that grows past ~5 MB of serialized `nodes`. A moderately-active day of agent usage (as observed today) is sufficient. Two sessions were already past this threshold at midnight and continued growing all day.
