# Spec: Scrollback Buffer Compaction

**Status:** Planned
**Branch:** agenta/scrollback-compaction
**Related:** Tier 3 scroll fix (PR #208), RAF write logging (PR #210)

---

## Problem

The scroll flash reappears once the terminal buffer reaches the xterm.js scrollback limit (default 2000 lines). Root cause confirmed via `[raf-write]` logs — flash onset correlates exactly with `bufLines` crossing 2000.

When the buffer is full, **every single `terminal.write()` call triggers a buffer trim**: xterm.js removes the oldest line(s) from the top and shifts all internal line numbers. This trim interacts with Ink's cursor-up/content sequence pattern:

1. Ink sends cursor-up (`ESC[XA`, 3 bytes) — arrives in RAF frame N, triggers write + trim → viewport snaps UP
2. Ink sends content (~200–500 bytes) — arrives in RAF frame N+1, triggers another write + trim → viewport snaps DOWN
3. DWM on Windows 10 presents both intermediate states as distinct frames → visible flash

Before the buffer is full (bufLines < 2000), no trim occurs per write, so the cursor-up snap is the only movement and it corrects within the same frame or is fast enough to be invisible.

---

## Solution: High-water mark compaction

Instead of trimming on every write, accumulate up to 5000 lines, then compact back to 2000 in a single scheduled operation. This reduces trim frequency from **every write** to **once per ~3000 new lines**.

```
scrollback: 5000 (high-water mark)
            ↓
     buffer grows 0 → 5000 (no per-write trim in this range)
            ↓
     bufLines >= 5000 → compact: set scrollback = 2000, then restore to 5000
            ↓
     buffer jumps from 5000 back to ~2000 (one-time trim, scheduled in idle RAF)
            ↓
     repeat
```

The one-time compaction trim will cause a viewport jump, but it happens **rarely** (every ~3000 lines of output) and can be scheduled when the cursor is at the bottom, minimizing visual impact.

---

## Implementation

### 1. `frontend/app/view/term/term.tsx`

Change the default scrollback from 2000 to 5000:

```ts
let termScrollback = 5000; // was 2000
```

The cap stays at 50000:
```ts
termScrollback = Math.max(0, Math.min(termScrollback, 50000));
```

### 2. `frontend/app/view/term/termwrap.ts`

Add a compaction threshold constant and a `compactBuffer()` method:

```ts
// Compact when the buffer exceeds this; trim back to SCROLLBACK_COMPACT_TARGET.
// Prevents per-write xterm.js trim (which causes viewport snaps) by doing one
// scheduled bulk trim every ~3000 lines instead.
private static readonly SCROLLBACK_HIGH_WATER = 5000;
private static readonly SCROLLBACK_COMPACT_TARGET = 2000;
private compactionPending = false;

private scheduleCompaction() {
    if (this.compactionPending) return;
    this.compactionPending = true;
    requestIdleCallback(() => {
        this.compactionPending = false;
        const lines = this.terminal.buffer.active.length;
        if (lines < TermWrap.SCROLLBACK_HIGH_WATER) return;
        // Setting scrollback lower than current buffer length triggers xterm.js trim.
        this.terminal.options.scrollback = TermWrap.SCROLLBACK_COMPACT_TARGET;
        // Restore high-water capacity immediately so future writes don't trim per-write.
        this.terminal.options.scrollback = TermWrap.SCROLLBACK_HIGH_WATER;
        console.log(`[scrollback] compacted ${lines} → ${this.terminal.buffer.active.length} lines`);
    }, { timeout: 500 });
}
```

In `scheduleRafWrite`, after the flush, check whether to schedule compaction:

```ts
this.doTerminalWrite(merged, null).then(() => {
    // ... existing timing log ...
    if (this.terminal.buffer.active.length >= TermWrap.SCROLLBACK_HIGH_WATER) {
        this.scheduleCompaction();
    }
});
```

### 3. Fallback for environments without `requestIdleCallback`

`requestIdleCallback` is not available in all WebView2 builds. Use a `setTimeout` fallback:

```ts
const scheduleIdle = (cb: () => void, timeout: number) => {
    if (typeof requestIdleCallback !== "undefined") {
        requestIdleCallback(cb, { timeout });
    } else {
        setTimeout(cb, timeout);
    }
};
```

---

## Trade-offs

| Aspect | Before (2000 limit) | After (5000 HWM + 2000 compact) |
|--------|--------------------|---------------------------------|
| Memory | ~2000 lines buffered | ~2000–5000 lines buffered |
| Trim frequency | Every write when full | ~Once per 3000 new lines |
| Flash cause | Constant per-write trim | Single bulk trim (rare) |
| Compaction flash | n/a | One viewport jump per compaction (idle-scheduled) |

Memory cost: each xterm.js line is roughly 80–200 bytes of JS object overhead. 5000 lines ≈ 0.5–1 MB additional — acceptable.

---

## Testing

1. Run Claude Code in a terminal pane with the fix applied
2. Confirm no flash before `bufLines = 5000`
3. At `bufLines = 5000`, watch for the single compaction log: `[scrollback] compacted 5000 → ~2138 lines`
4. Confirm no flash resumes after compaction

Tail the log:
```bash
tail -f ~/.agentmux/logs/agentmux-host-v*.log | grep -E 'raf-write.*SLOW|scrollback'
```

---

## Out of scope

- User-configurable high-water mark (can add later via `term:scrollback-hwm` setting)
- Smooth animated compaction
- Preserving exact scroll position across compaction (cursor stays at bottom; scrollback history above 2000 is intentionally discarded)
