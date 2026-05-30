# SPEC: Terminal double-rAF tear-out (experiment)

**Status:** Experimental — behind a default-off setting
**Date:** 2026-05-30
**Author:** AgentY
**Setting:** `term:disablerafcoalesce` (boolean, default `false`)
**Tracks:** [`SPEC_INPUT_RESPONSIVENESS_*`](./SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) · [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161) · bench PR #1203

---

## TL;DR

The terminal PTY→screen path runs **two `requestAnimationFrame` coalescers in series**:

```
PTY msg → [our Stage-1 rAF: termwrap.scheduleRafWrite/armRaf] → terminal.write()
        → [xterm's own rAF: RenderDebouncer] → paint
```

xterm.js already renders at most **once per animation frame** (its `RenderDebouncer`
guards on `_animationFrame` and merges dirty rows). Our Stage-1 rAF is therefore a
**second, unsynchronized frame gate**. Two gates in series can add up to ~32 ms and
**beat against each other** — the likely cause of the uneven frame pacing ("not
silky" hiccups) measured while holding a key: ~56 fps, frame p50 16.7 ms with a
33–50 ms tail, **0 JS long-tasks**, sub-ms xterm marks (PR #1203 diagnostic).

VS Code (same xterm.js, same Chromium) uses **only** xterm's built-in debouncer.

This change adds `term:disablerafcoalesce`. When set, `scheduleRafWrite()` writes
**straight to `terminal.write()`** and lets xterm own coalescing — removing our
Stage-1 rAF. Default is unchanged behavior (coalesce on).

## Why we have the Stage-1 rAF (don't delete blindly)

Full provenance — it fixed real bugs, in four commits:

| Commit | PR | Author | What |
|---|---|---|---|
| `ab9c38d6` | #208 (2026-03-22) | AgentA | **Introduced it.** Ink TUIs emit cursor-up + content as separate WS messages; two `terminal.write()`s snapped the viewport up-then-down = **double flash**. On **Windows 10** DWM presents each snap as a distinct frame; **on Windows 11 they coalesce within vsync (invisible).** Stage-1 rAF merges same-frame chunks into one write. |
| `7f442735` | #235 | AgentA | Added `writeInFlight` guard — a slow write (large scrollback) let a second concurrent rAF write reintroduce the flash. |
| `5bcf503f` | #276 | AgentA | Added the ≤512 B **fast-path** — the rAF was taxing echo; small writes bypass it. |
| `e58707766` | #926 | AgentY | Removed `writeInFlight` from the fast path — it still stalled echo during big writes ("sporadic jitter"). |

The arc #235→#276→#926 is a **progressive retreat**: each step carved the echo
path back *out* of the coalescer because the extra frame gate kept causing
latency. The tear-out is the logical end of that arc — but the **original Win10
flash is the load-bearing reason it exists**, so removal must be Win10-verified.

## Behavior

- `term:disablerafcoalesce` absent/`false` (default): unchanged. ≤512 B echoes
  fast-path; larger/streamed chunks buffer into our rAF, merged once per frame.
- `term:disablerafcoalesce: true`: every PTY write goes **directly** to
  `terminal.write()`; xterm's `RenderDebouncer` is the only frame gate.

Wired: `term.tsx` reads the setting → `TermWrap` `coalesceWrites` option →
early-return branch in `scheduleRafWrite()`. Plumbed through schema, gotypes,
and `wconfig` so the key round-trips.

## Verification protocol (the point of the flag)

A/B on the **same build** (no rebuild — just edit `settings.json`, settings hot-reload):

### Test 1 — does the Windows-10 scroll-flash regress? (BLOCKING)
This is the reason the coalescer exists. **Must run on a real Windows 10 machine.**
1. Open a terminal, run an Ink TUI that redraws heavily (e.g. a Claude Code / Codex
   session, or any full-screen `ink`/`blessed` app that emits cursor-up + content).
2. Baseline: `term:disablerafcoalesce` unset. Note scroll stability.
3. Tear-out: set `term:disablerafcoalesce: true`, reload, repeat the same workload.
4. **PASS** = no viewport up/down flash in tear-out mode. **FAIL** = flash returns →
   xterm's debouncer alone is insufficient; do NOT flip the default (consider a
   smaller targeted merge instead of a full second rAF).

### Test 2 — do the typing hiccups improve?
On both Win10 and Win11:
1. Focus a terminal, run `node tools/tests/term-keyrepeat-hiccups.mjs --secs 18`
   while holding a key, with the flag OFF → record VALID baseline.
2. Set the flag ON, reload, repeat.
3. Compare jank-frame counts (>33 ms) and the p99/max frame tail. Expect the
   tail to shrink if the double-rAF beat was the cause.

### Test 3 — streaming throughput / ordering (no regression)
`node tools/tests/bench-term-echo.mjs --stream --busy` with flag on vs off —
confirm no new ordering violations and no echo-latency regression under load.

## Rollout

If Test 1 passes on Win10 AND Tests 2–3 show improvement with no regression,
a follow-up flips the default (or removes Stage-1 entirely). Until then this ships
**default-off** — pure opt-in, safe for all users.
