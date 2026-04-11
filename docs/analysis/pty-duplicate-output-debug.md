# PTY Duplicate Output & Color Offset Debug — 2026-04-03

## Symptom
- Duplicate characters in xterm output (some chars duplicated, others dropped)
- Color highlighting at wrong offsets
- Observed with Claude Code CLI running inside AgentMux 0.33.27

## Root Cause Candidates

### 1. ptyOffset double-counting (most likely for duplicate/drop)

`doTerminalWrite()` in `termwrap.ts:381` tracks `ptyOffset` by adding `data.length` (byte count of the raw base64-decoded chunk). This is the **byte offset into the backend file**.

The load path in `loadInitialTerminalData()` does:
1. Fetch cache file → `doTerminalWrite(cacheData, ptyOffset)` — sets `ptyOffset` to the cache's stored value
2. Fetch main file from `ptyOffset` — appends only bytes after the cache

If there's a race or overlap between:
- `loadInitialTerminalData()` (initial load, uses `doTerminalWrite` with explicit `setPtyOffset`)
- `handleNewFileSubjectData()` (live WS events, uses `scheduleRafWrite` → `doTerminalWrite` with `setPtyOffset=null`)

…then live events that arrive **during** the load get written twice: once from the file fetch and once from the WS stream. Or they get dropped if the ptyOffset overshoots.

### 2. RAF bypass race (duplicate for small chunks)

`scheduleRafWrite()` at line 338-347:
```ts
if (data.length <= 512 && this.rafBuffer.length === 0 && !this.writeInFlight) {
    this.doTerminalWrite(data, null);  // direct write, bypasses RAF
    return;
}
this.rafBuffer.push(data);
```

If `writeInFlight=true` but `rafBuffer` is empty when a small chunk arrives, it falls through to `rafBuffer.push`. When the in-flight write finishes and drains the buffer, that chunk gets written. But if ANOTHER small chunk arrives before `writeInFlight` clears, the check `rafBuffer.length === 0` is false — it queues instead of bypasses. This is correct behavior, but it means ordering depends on the exact timing of `writeInFlight` transitions.

The guard added in PR #278 (`bf78e71`) protects against out-of-order small chunks, but a subtle case exists: if `doTerminalWrite` is called directly (bypass path) while a RAF flush is mid-`terminal.write()` callback, xterm.js may process them out of order internally since `terminal.write()` is async.

### 3. Wrong offsets for color (ANSI escape sequence split across chunks)

ANSI escape sequences (color codes) are multi-byte: `\x1b[38;2;R;G;Bm`. If a chunk boundary lands **inside** an escape sequence, xterm.js will:
- Process the partial sequence as literal characters → duplicate/garbage output
- When the continuation arrives, try to interpret it as a new sequence → wrong color offsets

Claude Code uses Ink which emits heavy ANSI: cursor positioning, RGB colors, cursor-up sequences. These can easily be split across WebSocket messages.

The RAF batching (PR #273) was specifically designed to coalesce these — but the bypass path for small chunks (≤512 bytes) can re-introduce splits if a continuation chunk is small and bypasses while the first chunk is in-flight.

### 4. `ptyOffset` uses `data.length` (bytes) but xterm uses character positions

`ptyOffset += data.length` counts **raw bytes** (the base64-decoded wire bytes). This is the correct unit for the backend file offset. However, if the offset drifts (e.g. due to the race in candidate #1), subsequent fetches request data from the wrong position → partial writes or overlap → duplicates or drops.

---

## Debug Instrumentation Plan

### A. Log every write with sequence number + offset tracking

In `doTerminalWrite()`, add a sequence counter and log before and after:

```ts
private writeSeq = 0;

doTerminalWrite(data: string | Uint8Array, setPtyOffset?: number): Promise<void> {
    const seq = ++this.writeSeq;
    const byteLen = typeof data === "string" ? data.length : data.length;
    const offsetBefore = this.ptyOffset;
    console.log(`[tw] write#${seq} bytes=${byteLen} ptyOffset=${offsetBefore} setPtyOffset=${setPtyOffset ?? "null"} src=${new Error().stack?.split('\n')[2]?.trim()}`);
    // ... existing code ...
    this.terminal.write(data, () => {
        console.log(`[tw] write#${seq} DONE ptyOffset=${this.ptyOffset} elapsed=${...}ms`);
        resolve();
    });
}
```

This tells you:
- Sequence of writes
- Whether offset is advancing correctly
- Whether any writes fire out of order (seq gaps)
- Call site (RAF flush vs direct bypass vs initial load)

### B. Tag the source of each write

Three paths feed `doTerminalWrite`:
1. `loadInitialTerminalData` (cache replay)
2. `loadInitialTerminalData` (main file fetch)
3. `scheduleRafWrite` → RAF flush
4. `scheduleRafWrite` → direct bypass

Add a `source` tag:

```ts
doTerminalWrite(data: string | Uint8Array, setPtyOffset?: number, source = "unknown"): Promise<void> {
    console.log(`[tw:${source}] seq=${++this.writeSeq} bytes=${data.length} ptyOffset=${this.ptyOffset}`);
```

Call sites:
- `doTerminalWrite(cacheData, ptyOffset, "cache")` 
- `doTerminalWrite(mainData, null, "main-file")`
- `doTerminalWrite(data, null, "raf-bypass")`
- `doTerminalWrite(merged, null, "raf-flush")`

### C. Detect chunk boundary ANSI splits

Check if a chunk ends mid-escape-sequence:

```ts
function endsInPartialEscape(data: Uint8Array): boolean {
    // Check last 8 bytes for \x1b or partial CSI
    const tail = data.slice(-8);
    for (let i = tail.length - 1; i >= 0; i--) {
        if (tail[i] === 0x1b) return true; // ESC not followed by complete sequence
    }
    return false;
}
```

Log a warning in `scheduleRafWrite` when a bypassed chunk ends in a partial escape — that's a guaranteed color offset bug.

### D. WS message arrival timestamps

In `handleNewFileSubjectData`, log arrival time and chunk size:

```ts
handleNewFileSubjectData(msg: WSFileEventData) {
    const now = performance.now();
    if (msg.fileop === "append") {
        const decoded = base64ToArray(msg.data64);
        console.log(`[tw:ws] fileop=append bytes=${decoded.length} t=${now.toFixed(1)}ms loaded=${this.loaded}`);
        // Check for ANSI split
        if (endsInPartialEscape(decoded)) {
            console.warn(`[tw:ws] WARN: chunk ends in partial escape — color split likely bytes=${decoded.length}`);
        }
```

This catches the key case: chunks arriving before `loaded=true` go into `heldData` — if they overlap with `loadInitialTerminalData`'s file fetch, you'll see duplicates.

### E. Log `loaded` transition

```ts
// Where this.loaded is set to true (in initTerminal):
console.log(`[tw] loaded=true, flushing ${this.heldData.length} held chunks`);
```

---

## Where to Look First

Given the "duplicating some, dropping others" behavior specifically with Claude Code output:

1. **Check the `loaded` flag race** — Claude Code streams output continuously. If WS events arrive before `loadInitialTerminalData` completes AND the file fetch already includes those bytes, you'll see exact duplicates of the tail of the output.

2. **Check `ptyOffset` after load** — add `console.log('[tw] initial load complete, ptyOffset=', this.ptyOffset)`. If it doesn't match the actual file size, every subsequent live write will be off.

3. **Check RAF bypass during high-throughput** — Claude Code's Ink UI emits many small cursor-positioning sequences. Some will hit the bypass path while larger content chunks are in-flight → potential ordering issue.

---

## Files to Modify

| File | What |
|------|------|
| `frontend/app/view/term/termwrap.ts` | Add debug logging in `doTerminalWrite`, `scheduleRafWrite`, `handleNewFileSubjectData`, `loadInitialTerminalData` |

All logs appear in the `task dev` terminal output tagged `[fe]` — **not in browser DevTools** (log pipe forwards all console output to the backend).

## Build & Test
```bash
npm run build:dev   # or task dev for hot reload
```
Look for `[fe] [tw:*]` entries in the sidecar log output.
