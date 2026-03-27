# Retro: Win10 Scroll Flash Investigation

**Date range:** ~v0.32.73 – v0.32.88+
**Status:** Monitoring — no flash observed in recent sessions (long testing needed to close)
**PRs:** #206, #208 (merged) · #210, #215 (closed, superseded) · #227 (open, pending merge)

---

## Problem

Terminal panes showed a visible flash/jump during Claude Code streaming output on Windows 10.
The DWM compositor was presenting intermediate cursor positions as distinct frames, causing
the viewport to snap up then back down — visible as a 1-2 frame flicker.

Reproducible: run any streaming CLI (Claude Code, Codex, Gemini) in a terminal pane and
watch the output scroll area. More pronounced at high scrollback buffer fill.

---

## Root Cause Chain (three separate causes found)

### Cause 1 — Ink double-write (Tier 1 + 2 fix, PR #206)
Claude Code uses [Ink](https://github.com/vadimdemedes/ink) for its rendering. Ink sends:
1. Content chunk (new text)
2. A small cursor-move sequence to reposition the cursor

When these arrived as separate WebSocket messages, each triggered a `doTerminalWrite()` call
and therefore two xterm.js viewport syncs in the same frame. Two syncs = two intermediate
states = flash.

**Fix (PR #206):** Input coalescing (Tier 1) and read-ahead buffering (Tier 2) to reduce
the number of discrete writes hitting xterm.js per frame.

### Cause 2 — Single RAF window not wide enough (Tier 3 + 4 attempts)
Even with coalescing, Ink's cursor-up (`ESC[A`) arrived 28–33ms after the content chunk —
after the 16ms RAF window had already flushed. So two separate RAF cycles fired:
- Frame N: content written → viewport syncs to new cursor position
- Frame N+2: cursor-up written → viewport snaps up → DWM captures this as a frame → flash

**Fix (PR #208, merged):** Tier 3 — `requestAnimationFrame`-batched writes. All data
received within a single 16ms frame is coalesced into one `terminal.write()` call.

**Attempted follow-on (PR #215, closed):** Tier 4 — widen the RAF window to 40ms via a
spin-wait loop (`tryFlush` inside `requestAnimationFrame`). This would catch the late
cursor-up. Closed because: (a) the 40ms hold adds perceptible latency on fast output,
(b) the `writeInFlight` approach in #227 is a cleaner fix for the same symptom.

### Cause 3 — Concurrent RAF writes at high scrollback fill (writeInFlight fix, PR #227)
At large scrollback buffer sizes (1000+ lines), `terminal.write()` can take >16ms
(the xterm.js scroll-trim logic runs synchronously during the write callback). The
original `scheduleRafWrite` set `rafPending = false` before `doTerminalWrite` resolved.
New data arriving during that window would schedule a second RAF, and both writes would
run concurrently — the second one capturing an intermediate viewport state → flash.

**Fix (PR #227, open):** `writeInFlight` flag + `armRaf()` helper. A new RAF is only
scheduled if both `rafPending` and `writeInFlight` are false. After each write resolves,
`armRaf()` is called again to drain any data that accumulated during the slow write.

Logs from a live session confirmed this: `[raf-write] SLOW` lines showed `elapsed=14-18ms`
at `bufLines=1800+`, while the basic 16ms Tier 3 guard was still active.

### Why scrollback raise was attempted (PR #215, closed)
Log analysis from the v0.32.74 session showed `bufLines=2138` (constant). At the old
default of 2000 lines, xterm.js trims the buffer on every single write — the trim itself
causes a viewport snap. The fix was to raise the default scrollback limit from 2000→10000
so the trim threshold is never reached in practice.

**Current status:** This change was not merged. The `writeInFlight` fix (PR #227)
prevents concurrent writes at high bufLines, which was the observable consequence. The
scrollback raise is still worth doing separately for user experience (more history) but
is not a scroll-flash fix on its own.

---

## What's in main

| PR | Version | Change | Status |
|----|---------|--------|--------|
| #206 | v0.32.73 | Tier 1+2: input coalescing + read-ahead | Merged |
| #208 | v0.32.73 | Tier 3: RAF-batched writes (16ms window) | Merged |
| #210 | — | Logging only (superseded by #227) | Closed |
| #215 | — | 40ms RAF window + scrollback 2k→10k | Closed (stale, competing approach) |
| #227 | — | `writeInFlight` guard + RAF timing logs | Open, awaiting merge |

---

## What PR #227 adds

**`frontend/app/view/term/termwrap.ts`:**

```
scheduleRafWrite()
  └─ pushes to rafBuffer
  └─ calls armRaf()

armRaf()  [NEW]
  └─ guards: if rafPending || writeInFlight → return
  └─ sets rafPending = true
  └─ requestAnimationFrame → flush
        sets writeInFlight = true  [NEW]
        calls doTerminalWrite().then(() => {
            writeInFlight = false  [NEW]
            armRaf()  // drain anything that buffered  [NEW]
            logs [raf-write] chunks/bytes/elapsed/bufLines  [NEW]
        })
```

No other files changed. Frontend-only, no Rust changes.

---

## Monitoring

After PR #227 merges, watch for `[raf-write] SLOW` in the host log:

```bash
tail -f ~/.agentmux/logs/agentmux-host-v*.log | grep raf-write
```

A `SLOW` line means `terminal.write()` took >8ms. If flash reappears, cross-reference
`bufLines` at the time — if it's near the scrollback limit (2000 default), consider
raising the default scrollback to 10000 as a separate PR.

---

## Current Status (as of 2026-03-26)

No flash observed in recent sessions running inside AgentMux v0.32.88.
The Tier 3 RAF batching (PR #208) appears to have been the dominant fix.
PR #227's `writeInFlight` guard is a correctness fix for a race that *can* occur at
high buffer fill — it hasn't been needed in current testing but closes the theoretical
window. Long sessions (hours, multiple Claude Code conversations) are needed to rule out
the race definitively.

**Next steps:**
1. Merge PR #227
2. Run a long session (4+ hours, multiple Claude Code conversations)
3. If no `[raf-write] SLOW` with bufLines > 1500 → close issue
4. If flash returns → check `bufLines` in logs to determine next action
