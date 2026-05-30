# SPEC: Remove the terminal Stage-1 RAF write-coalescer (double-rAF)

**Status:** Implemented — pending Windows-10 verification before release
**Date:** 2026-05-30
**Author:** AgentY
**PR:** #1211 (supersedes #1206, which added an opt-in flag — now removed)
**Tracks:** [`SPEC_INPUT_RESPONSIVENESS_*`](./SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) · [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161) · bench PR #1203

---

## TL;DR

The terminal PTY→screen path ran **two `requestAnimationFrame` coalescers in series**:

```
PTY msg → [our Stage-1 rAF: termwrap.scheduleRafWrite/armRaf] → terminal.write()
        → [xterm's own rAF: RenderDebouncer] → paint
```

xterm.js already renders **at most once per animation frame** (its `RenderDebouncer`
guards on `_animationFrame` and merges dirty rows). Our Stage-1 rAF was therefore a
**second, unsynchronized frame gate** — it added latency and **beat against** xterm's
rAF, producing the uneven frame pacing ("not silky" hiccups) measured while holding a
key: ~56 fps, frame p50 16.7 ms with a 33–50 ms tail, **0 JS long-tasks**, sub-ms
xterm marks (PR #1203 diagnostic). VS Code (same xterm.js, same Chromium) uses **only**
xterm's built-in debouncer.

**This change removes the Stage-1 rAF entirely.** There is no second coalescer and no
setting — *we don't need two.* PTY output writes straight to `terminal.write()`
(`termwrap.doTerminalWrite`); xterm's `RenderDebouncer` is the single frame gate.

## History — why it existed, and the two-step removal

The Stage-1 rAF fixed real bugs, in a four-commit arc:

| Commit | PR | What |
|---|---|---|
| `ab9c38d6` | #208 | **Introduced it.** Ink TUIs emit cursor-up + content as separate WS messages; two `terminal.write()`s snapped the viewport up-then-down = **double flash**. On **Windows 10** DWM presents each snap as a distinct frame; **on Windows 11 they coalesce within vsync (invisible).** |
| `7f442735` | #235 | `writeInFlight` guard — a slow write (large scrollback) let a second concurrent rAF write reintroduce the flash. |
| `5bcf503f` | #276 | ≤512 B echo **fast-path** — the rAF was taxing echo; small writes bypass it. |
| `e58707766` | #926 | removed `writeInFlight` from the fast path — it still stalled echo during big writes ("sporadic jitter"). |

#235→#276→#926 was a **progressive retreat** carving the echo path back *out* of the
coalescer because the extra frame gate kept causing latency. PR #1206 then added an
opt-in `term:disablerafcoalesce` flag to A/B the full bypass — confirmed **"much
better"** on Windows 11. **This change is the end of that arc:** the flag and the
entire Stage-1 machinery are deleted. One coalescer, not two; no dead toggle.

## What changed (this PR)

`frontend/app/view/term/termwrap.ts`:
- Deleted fields `rafBuffer`, `rafPending`, `writeInFlight`, and `coalesceWrites`.
- Deleted `RAF_BYPASS_THRESHOLD`, `scheduleRafWrite()`, and `armRaf()`.
- `handleNewFileSubjectData` now calls `doTerminalWrite(decodedData)` directly,
  preserving the `term-echo-render` perf mark for ≤32 B writes (closed in
  `doTerminalWrite`'s write callback).

Setting removal (it had exactly one reader, now gone):
- `frontend/app/view/term/term.tsx` — dropped the `coalesceWrites` option.
- `frontend/app/view/term/termwrap.ts` — dropped `coalesceWrites` from `TermWrapOptions`.
- `schema/settings.json`, `frontend/types/gotypes.d.ts`,
  `agentmux-srv/src/backend/wconfig/types.rs` — removed `term:disablerafcoalesce`.

Perf marks: `term-keypress` and `term-echo-render` unchanged; `term-raf-write` is
gone (there is no rAF write). Net **−96 lines** across 6 files.

Build verified: `npx tsc --noEmit` — no new errors (27-error pre-existing baseline
unchanged; none in `termwrap.ts`/`term.tsx`). `cargo check -p agentmux-srv` — clean.
`schema/settings.json` valid JSON.

## ⚠️ Windows-10 verification — REQUIRED before this ships in a release

The Stage-1 rAF existed specifically to fix the **Windows-10 DWM scroll-flash** (PR
#208), which is invisible on Windows 11. So the Win11 "much better" result does **not**
clear Win10. On **real Windows 10**:

### Test 1 — flash regression (BLOCKING)
1. Open a terminal, run an Ink TUI that redraws heavily (a Claude Code / Codex session,
   or any full-screen `ink`/`blessed` app emitting cursor-up + content).
2. Compare a build **with** this change against one **without** it (or the last release).
3. **PASS** = no viewport up/down flash with the rAF removed. **FAIL** = flash returns →
   xterm's debouncer alone is insufficient; do not ship; see "If it regresses" below.

### Test 2 — hiccup delta (confirms the win)
`node tools/tests/term-keyrepeat-hiccups.mjs --secs 18` (PR #1203) while holding a key,
before vs after. Expect the >33 ms jank count and the p99/max frame tail to shrink.

### Test 3 — streaming throughput / ordering (no regression)
`node tools/tests/bench-term-echo.mjs --stream --busy` before vs after — no new ordering
violations, no echo-latency regression under load.

**Theory it passes even on Win10:** xterm's `RenderDebouncer` renders only once per
animation frame, so the intermediate cursor-up/content viewport state should never paint
as a separate frame even when the two writes arrive separately. The #208 flash came from
*our* path issuing two `terminal.write()`s that each forced a viewport sync before
xterm's debounced render; removing our coalescer leaves xterm's single debounced render
in charge. Test 1 confirms or refutes this.

## If the Win10 flash regresses

The correct fix is **backend-side read coalescing**, NOT a second frontend rAF: in the
PTY read loop (`agentmux-srv/src/backend/blockcontroller/shell.rs` — `spawn_blocking`,
`PTY_READ_BUF_SIZE` 4 KiB, `handle_append_block_file(..., None)`), merge consecutive
reads within a small window (~2–4 ms) into one `block:file` append event. That kills the
flash at the source (the root cause per #208 is *"PTY data arrives as separate WS
messages"*) and works on both platforms without re-introducing a per-frame beat. This
frontend removal composes with it.
