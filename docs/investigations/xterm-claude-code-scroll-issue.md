# xterm.js + Claude Code: Unexpected Scroll Investigation

**Date:** 2026-03-22
**Status:** Research complete — mitigations identified
**Scope:** Why Claude Code causes xterm.js to scroll unexpectedly, and what AgentMux can do about it

---

## Problem Statement

When Claude Code (or any Ink-based CLI) runs inside an AgentMux terminal pane, the xterm.js viewport repeatedly jumps — scrolling the entire screen — without user interaction. This is most visible during streaming output and tool execution (spinners, status lines). It is one of the most-reported xterm.js user-experience issues: [claude-code#826](https://github.com/anthropics/claude-code/issues/826) has 634 upvotes and 337 comments.

---

## Root Causes

### 1. Ink's "Erase-and-Redraw" Rendering Architecture (Primary)

Claude Code is built on React + [Ink](https://github.com/vadimdemedes/ink). On every React state change (streaming token, spinner tick, status bar update), Ink's renderer executes:

```javascript
stream.write(ansiEscapes.eraseLines(previousLineCount) + newOutput)

// eraseLines implementation:
eraseLines = (count) => {
  let str = "";
  for (let i = 0; i < count; i++)
    str += eraseLine + (i < count - 1 ? cursorUp() : "");
  if (count) str += cursorLeft;
  return str;
}
```

When the UI is 60 lines tall, this generates 60 pairs of `ESC[2K` (erase line) + `ESC[1A` (cursor up), followed by the full new content redrawn from scratch. This is **not** a diff — it is a complete erase and full redraw every render cycle.

**The xterm.js problem:** When the cursor moves up via repeated `ESC[1A`, xterm.js scrolls the viewport to track the cursor, jumping far up into the scrollback buffer. Then when new content is written and the cursor returns to the bottom, the viewport snaps back down. This fires multiple times per second because Ink re-renders for every streamed token, every spinner tick, and every status line update — concurrently.

### 2. Specific CSI/ANSI Sequences That Trigger Viewport Changes

Beyond cursor-up, these sequences also cause viewport jumps:

| Sequence | Name | Effect |
|---|---|---|
| `ESC[H` | CUP (Cursor Home) | Moves cursor to 0,0 — xterm.js scrolls to top |
| `ESC[2J` | ED2 (Erase Display) | Clears screen; when `scrollOnEraseInDisplay` is on, pushes to scrollback |
| `ESC[r` | DECSTBM (Set Scroll Region) | xterm.js repositions viewport when margins change |
| `ESC[?1049h/l` | Alternate Screen Buffer | Complete viewport context switch |
| `ESC[S` | SU (Scroll Up) | Physically shifts viewport upward |
| `ESC c` | RIS (Full Reset) | Clears scrollback entirely ([xterm.js #3315](https://github.com/xtermjs/xterm.js/issues/3315)) |

### 3. xterm.js Scroll-on-Output (Partially Fixed)

Early xterm.js scrolled unconditionally to bottom on any output. [PR #336](https://github.com/xtermjs/xterm.js/pull/336) added a `userScrolling` flag that prevents scroll-to-bottom when the user has scrolled up. However, this only covers line-output scrolling — scrolling caused by **cursor movement sequences** (the Ink pattern) bypasses this flag. xterm.js always moves the viewport to track cursor position.

Additionally, user input always scrolled to bottom until [PR #4289](https://github.com/xtermjs/xterm.js/pull/4289) (xterm.js 5.1.0) added `scrollOnUserInput: false`.

### 4. macOS Trackpad Momentum Scroll Compounding

On macOS, after lifting a finger, the OS continues generating `WheelEvent`s with decaying `deltaY` values. When this fires simultaneously with Ink's cursor-up sequences (which move the viewport), the two compound into a "rocket scroll" feedback loop — oscillating between top and bottom at high speed.

### 5. Focus Events Causing Viewport Resets (Claude Code v2.1.5+ Regression)

[Issue #18299](https://github.com/anthropics/claude-code/issues/18299): A regression in CC v2.1.5 caused the TUI to reset scroll position on terminal focus-in/focus-out events, even when Claude Code is completely idle with no output. Every time the window gains or loses focus, the viewport jumps. Fixed in subsequent CC versions.

### 6. Windows Terminal Bug with Cursor Positioning (Windows-specific)

[microsoft/terminal#14774](https://github.com/microsoft/terminal/issues/14774): `SetConsoleCursorPosition` always scrolls the viewport to the cursor position — even when the cursor is already visible. This means Ink's cursor-up movement always causes visible viewport scrolling on Windows Terminal with no in-process workaround.

---

## What Anthropic Has Done

**Differential Renderer (January 2026):** Rewrote Ink's rendering layer to diff previous output and only emit changed lines. Significantly reduced `eraseLines` frequency. Does **not** eliminate the issue — any render that changes output height still emits cursor-up sequences.

---

## xterm.js Options That Affect Scroll Behavior

| Option | Default | Effect |
|---|---|---|
| `scrollOnUserInput` | `true` | Set `false` to prevent scroll-to-bottom on keystrokes (requires xterm.js ≥ 5.1.0) |
| `scrollOnEraseInDisplay` | `false` | When `true`, ED2 (`ESC[2J`) pushes content to scrollback |
| `smoothScrollDuration` | `0` | Keep at `0` — animated scroll makes cursor jumps more disorienting |
| `scrollback` | `1000` | Larger values worsen scroll jump visibility |
| `scrollSensitivity` | — | Wheel scroll multiplier |

**Critical gap:** There is **no option** to suppress cursor-tracking-induced viewport scrolling. This is baked into xterm.js's core rendering contract.

---

## xterm.js API for Interception

| API | Notes |
|---|---|
| `terminal.attachCustomWheelEventHandler(fn)` | Intercept wheel events before xterm processes them; return `false` to cancel |
| `terminal.parser.registerCsiHandler({ final: 'A' }, fn)` | Hook CSI sequences (e.g., cursor-up `ESC[A`) |
| `terminal.buffer.active.viewportY` | Current viewport scroll position |
| `terminal.scrollToLine(n)` | Programmatically restore scroll position |
| `terminal.modes.synchronizedOutputMode` | True when DEC 2026 is active |
| `terminal.onScroll` | **Warning:** Only fires on new lines added — NOT on user scroll |

---

## Mitigation Strategy (Tiered)

### Tier 1: xterm.js Options (Zero Risk — Do Now)

```typescript
// In termwrap.ts, add to terminal options:
scrollOnUserInput: false,   // Prevent scroll-to-bottom on keystrokes (xterm.js >= 5.1.0)
smoothScrollDuration: 0,    // No animated scroll (makes jumps less disorienting)
// scrollOnEraseInDisplay: leave at false (default)
```

AgentMux is on xterm.js 5.5.0, so `scrollOnUserInput` is available.

### Tier 2: Block Momentum Scroll (Low Risk)

```typescript
// In termwrap.ts constructor, after terminal creation:
this.terminal.attachCustomWheelEventHandler((ev) => {
    // Block macOS trackpad momentum scroll (tiny decaying deltaY values)
    if (Math.abs(ev.deltaY) < 4) return false;
    return true;
});
```

This eliminates the "rocket scroll" feedback loop on macOS without affecting normal scrolling.

### Tier 3: Verify DEC 2026 Synchronized Output Is Working

xterm.js 5.5.0 has native DEC 2026 support. Claude Code emits `ESC[?2026h`/`ESC[?2026l` markers. When working, entire re-renders arrive as one atomic write — intermediate cursor-tracking states are invisible.

**Verification:** Log `this.terminal.modes.synchronizedOutputMode` during Claude Code output. If it never toggles, Claude Code isn't detecting DEC 2026 support from the terminfo/`TERM` env.

### Tier 4: CSI Parser Hook to Preserve Scroll Position

xterm.js exposes a full parser hooks API ([Hooks Guide](https://xtermjs.org/docs/guides/hooks/)) to intercept CSI sequences. Can intercept cursor-up (`ESC[A`) and cursor-home (`ESC[H`) to restore user scroll position after cursor tracking:

```typescript
// In termwrap.ts:
let isUserScrolled = false;
let savedViewportY = 0;

this.terminal.attachCustomWheelEventHandler((ev) => {
    isUserScrolled = true;
    savedViewportY = this.terminal.buffer.active.viewportY;
    return true;
});

// Hook cursor-up (CSI A) — fires when Ink erases lines
this.terminal.parser.registerCsiHandler({ final: 'A' }, (params) => {
    if (isUserScrolled) {
        const saved = savedViewportY;
        // Restore after xterm processes the sequence
        Promise.resolve().then(() => this.terminal.scrollToLine(saved));
    }
    return false; // still process normally
});
```

**Note:** This causes visible flicker because the viewport jumps and snaps back. Better combined with Tier 5.

### Tier 5: RAF-Based Write Batching

Accumulate PTY data within one animation frame and flush once per frame. Reduces `terminal.write()` calls from hundreds/sec to ~60/sec, each causing fewer viewport events.

```typescript
// In termwrap.ts:
private pendingData: Uint8Array[] = [];
private rafPending = false;

private scheduleWrite(data: Uint8Array) {
    // Fast path: small writes (interactive input) bypass batching
    if (!this.rafPending && data.length < 256) {
        this.doTerminalWrite(data, null);
        return;
    }
    this.pendingData.push(data);
    if (!this.rafPending) {
        this.rafPending = true;
        requestAnimationFrame(() => {
            const merged = mergeUint8Arrays(this.pendingData);
            this.pendingData = [];
            this.rafPending = false;
            this.doTerminalWrite(merged, null);
        });
    }
}
```

### Tier 6: PTY-Level Differential Rendering (Most Effective)

The [claude-chill](https://github.com/davidbeesley/claude-chill) approach: run Claude Code through a PTY proxy that maintains an in-memory VT100 state machine, diffs each render frame, and passes only changed lines to the terminal — wrapped in DEC 2026 sync markers. Instead of 60× `ESC[2K ESC[1A]` + 60 lines redrawn, only the cells that actually changed reach xterm.js. The cursor never moves up into the scrollback zone.

For AgentMux, this could be implemented in the Rust backend (`agentmuxsrv-rs`) as a stream transform on the PTY output pipe — intercept and diff before forwarding over WebSocket. This is architecturally clean (zero frontend changes) but the most substantial implementation effort.

---

## Recommended Implementation Order

1. **Now:** Add `scrollOnUserInput: false` and `smoothScrollDuration: 0` to terminal options in `termwrap.ts`
2. **Short-term:** Add `attachCustomWheelEventHandler` to block momentum scroll
3. **Verify:** Check if DEC 2026 mode is already working (log `synchronizedOutputMode`)
4. **Medium-term:** Implement RAF batching for bulk PTY output
5. **Long-term:** Evaluate PTY-level differential rendering in the Rust backend

---

## Tracking Issues

| Issue | Description | Status |
|---|---|---|
| [claude-code #826](https://github.com/anthropics/claude-code/issues/826) | Console scrolling to top (634 upvotes, 337 comments) | OPEN |
| [claude-code #3648](https://github.com/anthropics/claude-code/issues/3648) | Terminal scrolling uncontrollably | CLOSED |
| [claude-code #18299](https://github.com/anthropics/claude-code/issues/18299) | Scroll position lost after v2.1.5 flickering fix | CLOSED |
| [claude-code #34794](https://github.com/anthropics/claude-code/issues/34794) | Terminal scrolls to top during agent execution (Windows) | OPEN |
| [xterm.js #1824](https://github.com/xtermjs/xterm.js/issues/1824) | scrollOnUserInput configurable | CLOSED (PR #4289) |
| [xterm.js #5453](https://github.com/xtermjs/xterm.js/pull/5453) | DEC mode 2026 synchronized output | MERGED |
| [microsoft/terminal#14774](https://github.com/microsoft/terminal/issues/14774) | SetConsoleCursorPosition always scrolls viewport | OPEN |
| [github/copilot-cli #1805](https://github.com/github/copilot-cli/issues/1805) | 4-layer rocket scroll fix reference | OPEN |

---

## Files to Modify

- `frontend/app/view/term/termwrap.ts`
  - `constructor()` — add `scrollOnUserInput`, `smoothScrollDuration` options; add wheel handler
  - `doTerminalWrite()` — optionally add RAF batching
  - `init()` — verify DEC 2026 mode logging

No backend changes required for Tiers 1–4. Tier 6 (differential rendering) requires `agentmuxsrv-rs` stream transform work.

---

## References

- [anthropics/claude-code #826](https://github.com/anthropics/claude-code/issues/826)
- [anthropics/claude-code #34794](https://github.com/anthropics/claude-code/issues/34794)
- [github/copilot-cli #1805 — 4-Layer Rocket Scroll Fix](https://github.com/github/copilot-cli/issues/1805)
- [claude-chill PTY proxy](https://github.com/davidbeesley/claude-chill)
- [xterm.js ITerminalOptions](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- [xterm.js Terminal API](https://xtermjs.org/docs/api/terminal/classes/terminal/)
- [xterm.js Parser Hooks Guide](https://xtermjs.org/docs/guides/hooks/)
- [xterm.js PR #5453 — DEC mode 2026](https://github.com/xtermjs/xterm.js/pull/5453)
- [xterm.js PR #4289 — scrollOnUserInput](https://github.com/xtermjs/xterm.js/pull/4289)
- [INK-ANALYSIS.md — Ink rendering pipeline](https://github.com/atxtechbro/test-ink-flickering/blob/main/INK-ANALYSIS.md)
- [Taming ANSI-Induced Scrolling — Termdock approach](https://app.daily.dev/posts/taming-ansi-induced-scrolling-in-xterm-js-termdock-s-80-fix-for-claude-code-jumping-wugci510e)
- [microsoft/terminal#14774 — Windows cursor positioning bug](https://github.com/microsoft/terminal/issues/14774)
