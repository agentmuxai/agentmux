# Terminal Input Latency — Investigation & Fix Report

**Date:** 2026-05-19  
**Version:** AgentMux 0.34.0  
**File:** `frontend/app/view/term/termwrap.ts`  
**PR Branch:** `agenty/term-input-latency-fix`

---

## Problem Statement

Users reported **inconsistent, sporadic delays when typing in terminal panes** — not a
consistent latency, but jitter. Some keypresses felt instant; others had a noticeable lag
of ~16ms or more. The pattern was especially pronounced while terminal output was actively
streaming (e.g. running a command with lots of output).

---

## Architecture: How a Keypress Travels

```
User types key
    │
    ▼
xterm.js onData fires → handleTermData()
    │
    ▼
sendDataHandler(data) → WebSocket JSON-RPC → Rust backend
    │
    ▼
Backend writes byte to PTY master FD (write_all)
    │
    ▼
Kernel echoes byte back through PTY
    │
    ▼
Backend reads PTY output → WebSocket message to frontend
    │
    ▼
handleNewFileSubjectData() → scheduleRafWrite()
    │
    ├── fast path: doTerminalWrite() immediately   ← goal: always here
    │
    └── slow path: rafBuffer.push() + armRaf()    ← was: sometimes here
            │
            ▼
        requestAnimationFrame callback fires (≤16ms later)
            │
            ▼
        doTerminalWrite() → xterm renders echo
```

**Round-trip stages and their latency budgets:**

| Stage | Expected | Notes |
|-------|----------|-------|
| xterm onData → WS send | < 1ms | Synchronous JS |
| WS → Rust backend | < 1ms | Local IPC |
| Rust PTY write + echo | 1–3ms | Kernel round-trip |
| WS → frontend | < 1ms | Local IPC |
| `scheduleRafWrite` → xterm render | **0ms (fast) or ≤16ms (slow)** | Root cause lives here |

**Total echo round-trip (healthy):** ~3–6ms  
**Total echo round-trip (degraded):** ~3–6ms + up to 16ms RAF wait = **up to ~22ms**

---

## Root Cause: `writeInFlight` Blocking the Echo Fast Path

### The Guard That Caused Jitter

`scheduleRafWrite()` had a fast path that bypassed the RAF queue for small data (≤512 bytes):

```typescript
// BEFORE (broken)
private scheduleRafWrite(data: Uint8Array) {
    if (data.length <= TermWrap.RAF_BYPASS_THRESHOLD
        && this.rafBuffer.length === 0
        && !this.writeInFlight) {           // ← this guard caused the problem
        this.doTerminalWrite(data, null);
        return;
    }
    this.rafBuffer.push(data);
    this.armRaf();
}
```

The `writeInFlight` flag is set to `true` while a large RAF-batched write is being processed
by xterm.js — which can take anywhere from a few ms (small output) to tens of ms (large
scrollback buffer or slow render path).

### The Failure Scenario

```
Timeline:
  t=0ms   Large PTY output arrives → rafBuffer=[bigChunk] → RAF fires
  t=0.5ms RAF callback: merge chunks → writeInFlight=true → terminal.write(bigBatch)
  t=1ms   User types 'a' → echo arrives (4 bytes) → scheduleRafWrite(4b)
            → fast path blocked: writeInFlight=true ← JITTER SOURCE
            → rafBuffer.push(4b)
            → armRaf() → blocked: writeInFlight=true → nothing
  t=8ms   bigBatch write completes → writeInFlight=false → armRaf() fires
  t=8ms   RAF scheduled (next animation frame)
  t=24ms  RAF fires → echo written → echo visible to user
            ↑ 23ms total echo latency instead of ~5ms
```

This explains the **inconsistency**: delays only appeared when large PTY output was being
processed simultaneously. During quiet terminal sessions, `writeInFlight` was almost always
false, and the fast path worked correctly.

### Why Removing the Guard Is Safe

xterm.js **serialises all `terminal.write()` calls internally** via an async queue. When you
call `terminal.write(data, callback)` while another write is in progress, xterm queues the
new call and processes it after the current one completes — in order, with correct callback
sequencing.

The only valid ordering concern would be: a small echo bypassing *buffered data already in
`rafBuffer`* that should appear first. This is why the `rafBuffer.length === 0` check is
retained. When the RAF buffer is empty, there is no buffered data that could be reordered —
the new small write goes directly into xterm's internal queue, after the in-flight write's
data, which is exactly the correct order.

```typescript
// AFTER (fixed)
private scheduleRafWrite(data: Uint8Array) {
    // writeInFlight intentionally NOT checked: xterm serialises write() calls internally,
    // so a concurrent small write always lands after the in-flight batch without ordering
    // issues. Removing the guard eliminates the ≤16ms RAF stall on echo rendering.
    if (data.length <= TermWrap.RAF_BYPASS_THRESHOLD && this.rafBuffer.length === 0) {
        this.doTerminalWrite(data, null);
        return;
    }
    this.rafBuffer.push(data);
    this.armRaf();
}
```

---

## Instrumentation Added

Three perf marks are now emitted on every keypress + echo cycle. They appear in:
- **CEF DevTools → Performance tab** (record a trace, look for `term-*` measures)
- **AgentMux Perf HUD** (Ctrl+Shift+P) → Interactions panel
- **Console** (marks > 100ms trigger a `[perf]` warning)

### `term-keypress`

Measures the time from xterm's `onData` callback to after `sendDataHandler` dispatches
the WS message. Should be <1ms. If this is slow, the bottleneck is in the WS send path.

```
markStart('term-keypress')    ← in handleTermData(), before sendDataHandler call
markEnd('term-keypress', 'sent')  ← after sendDataHandler?.(data)
```

### `term-echo-render`

Measures the full time from when the echo arrives over WebSocket to when xterm.js has
finished rendering it (callback fires). Only emitted for small writes (≤32 bytes) which
correspond to character echoes and single completions.

```
markStart('term-echo-render')      ← in handleNewFileSubjectData, on small append
markEnd('term-echo-render', 'rendered')  ← in doTerminalWrite write callback
```

**Healthy baseline:** ~0–4ms (fast path, no queuing)  
**Degraded (before fix):** up to 16ms + remaining write time

### `term-raf-write`

Measures the full RAF batch write — from when the RAF callback fires and calls
`doTerminalWrite(merged)` to when xterm's write callback resolves. This covers the time
xterm.js spends parsing and rendering the merged batch.

```
markStart('term-raf-write')    ← before doTerminalWrite(merged, null)
markEnd('term-raf-write', 'done')   ← in .then() after writeInFlight=false
```

Slow writes (>4ms) also log to console as `[raf-write] SLOW chunks=N bytes=M elapsed=Xms bufLines=Y`.

---

## How to Profile with CEF DevTools

1. Open AgentMux → click the `devtools` widget in the pinned bar (or Ctrl+Shift+I)
2. Switch to the **Performance** tab
3. Click **Record** (circle button)
4. Type ~50 characters in a terminal pane, including some during active command output
5. Stop recording
6. In the **Timings** lane, look for `term-keypress`, `term-echo-render`, `term-raf-write`
7. In the **Main** thread flame chart, look for long tasks (gray blocks ≥50ms) that
   coincide with slow `term-echo-render` measurements

### What to look for

| Observation | Diagnosis |
|------------|-----------|
| `term-echo-render` > 4ms only when `term-raf-write` is active | WriteInFlight bug (fixed by this PR) |
| `term-echo-render` > 4ms even when no RAF write is active | xterm internal queue backed up — investigate scrollback size |
| Long Task (≥50ms) correlating with slow echo | Chromium GC pause or heavy Ink rerender |
| `term-keypress` > 2ms | WS dispatch is slow — check `invokeCommand` IPC timing |
| `term-raf-write` > 16ms | RAF batch too large — consider splitting at 64KB |

---

## Secondary Candidates (Not Yet Confirmed)

### 1. Chromium GC Pauses

The LongTask observer (`frontend/perf/observers.ts`) fires on tasks ≥50ms. These would
appear in `muxlog host '\[perf\]'` as `[perf] longtask` entries. If GC pauses are frequent,
the options are:

- Reduce allocations in the hot write path (avoid creating merged `Uint8Array` for single-chunk writes)
- Tune V8's GC via CEF flags (`--js-flags=--max-old-space-size=512`)

### 2. Scrollback Buffer Size

`doTerminalWrite` write time scales with scrollback buffer depth. If `bufLines` in the
`[raf-write] SLOW` log line is >5000, consider:

- Lowering the default `scrollback` option (currently whatever xterm's default is)
- Increasing `MinDataProcessedForCache` to trigger cache serialisation more aggressively,
  which resets the buffer

### 3. WebSocket Backpressure

If the Rust backend is saturated with PTY output, the WS send queue could back up. This
would show as `term-keypress` measures consistently >2ms. Check with `muxlog srv` for
write queue depth warnings.

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/view/term/termwrap.ts` | Remove `writeInFlight` guard from fast path; add `term-keypress`, `term-echo-render`, `term-raf-write` perf marks; lower slow-write log threshold 8ms→4ms |
| `.changesets/1779148800-fix-term-echo-latency-writeflight-bypass-k8m2.md` | Patch changeset |

---

## Expected Impact

- **Sporadic ~16ms delays during active output:** eliminated. Echo now goes directly into
  xterm's write queue regardless of any in-flight large write.
- **Baseline echo latency:** unchanged (~3–6ms round-trip, dominated by PTY kernel latency)
- **Flicker regression risk:** none. The RAF batching for large multi-chunk writes is
  unchanged. Only the behaviour of concurrent small writes during a large write is affected.

---

## Next Steps

1. Merge the PR and build a dev package (`task package`)
2. Open a terminal, run a command with lots of output (`find / 2>/dev/null` or similar),
   and type while it streams — the jitter should be gone
3. Record a CEF DevTools Performance trace and check `term-echo-render` p99 < 8ms
4. If `term-raf-write` shows consistently >16ms with large `bufLines`, file a follow-up
   to cap scrollback at 10k lines or implement adaptive batch splitting
5. Watch `muxlog host '\[perf\]'` for any LongTask entries correlating with echo slowdowns —
   those would indicate a GC or renderer bottleneck requiring a separate investigation
