# RCA: opening many terminals blanks some and reloads the app three times

**Date:** 2026-10-13
**Status:** retro. Root cause confirmed from logs and the live page. Option 1 of §5, which stops the reload loop, is implemented in the PR that adds this document; option 2 is open.
**Author:** Camper
**Trigger:** Repo owner, 2026-10-13, in an isolated `task dev` build (`CamperTerm`, branch `camper/term-resize-no-blank-frame` on `main` @ `5d9af9260`): *"your instance reset a bunch of times after I opened a bunch of terminals. some of the terminals were white, it eventually recovered after about 3-4 reloads, which it did automatically without me doing anything."*

## 1. Summary

**Two mechanisms on `main`, each reasonable alone, loop together:**
1. **One WebGL context per terminal, with no limit.** Each terminal's xterm WebGL renderer (`termwrap.ts` `loadRendererAddon`) gets its own `webgl2` context. Chromium allows a page 16 live WebGL contexts. When a 17th is created, it logs `Too many active WebGL contexts. Oldest context will be lost.` and kills the oldest. Every terminal in the window counts, including those in hidden window tabs (kept laid out) and agent panes' shell drawers.
2. **A page-level handler that reloads the app on any lost WebGL context** (`frontend/bootstrap.ts`, #316, 2026-04-08). It was written for a GPU or driver reset. It can't tell that from an eviction.

**The loop:**
- With 21 terminals, the 17th–21st evicted 5 contexts. Those terminals went white, and the handler reloaded the page.
- The reload recreated all 21 terminals, evicting 5 again, and the page reloaded again.
- After 3 reloads the handler stops (`CONTEXT_LOSS_MAX_RELOADS`). Each evicted terminal's own `onContextLoss` then switched it to the DOM renderer. That is the "recovery".

**The terminal resize fix on the same branch played no part.** Contexts are counted when they are created, and that change creates none.

## 2. Timeline (dev build's `cef-debug.log`, local time, 2026-10-13)

| Time | Event |
|---|---|
| 09:01:43 | New terminals opening (`block:40569d37…` and others) |
| 09:01:44 | `Too many active WebGL contexts` ×5 → `webglcontextlost` → `[recovery] WebGL context lost — reloading page in 1s (attempt 1/3)` |
| 09:01:50 | After the reload: `Too many active WebGL contexts` ×5 → `reloading page in 1s (attempt 2/3)` |
| 09:01:54 | ×6 → `attempt 3/3` |
| 09:01:57 | ×6 → `suppressing reload (already reloaded 3x, possible driver issue)` |
| 09:02:00 | `WebGL context lost, falling back to DOM renderer` ×5 |
| 09:02:00 → 09:04:26 | No further context warnings; the page is stable |

**State afterwards, from the live page:** 21 terminals, 16 with a WebGL canvas, 5 on the DOM renderer. 21 − 16 = 5, the number of evictions per load.

## 3. Why the terminals were white

An evicted context's canvas has nothing drawn in it. The terminal shows as an empty, white canvas until it either falls back to the DOM renderer or the page reloads. During attempts 1–3 the page reloaded before the fallback mattered, so the white terminals were visible for about a second each time.

## 4. Impact

- **Who:** anyone with more than 16 terminals in one window, across all its tabs, agent shell drawers included. That is reachable in normal use: a few agents with their shells open plus a tab of terminals.
- **What they see:** some terminals go blank, then the whole window reloads three times in about 15 seconds. Every pane remounts and agent views re-render. Sessions survive (they live in srv), but it reads as a crash.
- **After the third reload:** the extra terminals run on the slower DOM renderer for the rest of the session. The recovery counter resets 60 s after a load, so opening one more terminal later can start the loop again.
- **The real GPU-reset case** the handler was written for is no better served: one evicted context is enough to trigger it.

## 5. Fix options

1. **Don't reload the page for a terminal's lost context.**
   - `termwrap.ts` already handles it per terminal (`onContextLoss` → dispose → DOM renderer). The global handler in `bootstrap.ts` should ignore a `webglcontextlost` whose target is inside `.xterm`, or be removed if nothing else needs it: it predates the per-terminal fallback.
   - **What it fixes:** the reload loop. Smallest change.
2. **Stay under the cap.** Give WebGL only to terminals that can be seen:
   - when a terminal's tab or drawer is hidden, dispose its WebGL renderer;
   - when it's shown again, recreate it;
   - with a ceiling (say 12) as a backstop, past which new terminals start on the DOM renderer.

   **What it fixes:** no terminal ever goes white from eviction, and contexts aren't spent on terminals nobody can see. Somewhat more work: the renderer swap on show and hide has to avoid a flash.
3. **Handle eviction as an expected event:** on a lost context, fall back to DOM for that terminal (already done), and move it back to WebGL when a context frees up.

**Recommendation:** option 1 now, since it stops the loop and is a few lines. Then option 2, so terminals beyond the cap don't go white and hidden terminals stop holding contexts.

**Done in this PR (option 1):**
- **`frontend/app/platform/webgl-context-loss.ts` `isTerminalContext`:** `bootstrap.ts` now ignores a lost context whose canvas is inside `.xterm`. Any other lost context is still treated as a GPU reset, as before.
- **What remains:** xterm handles the terminal's lost context itself. It calls `preventDefault()`, waits 3 s for `webglcontextrestored`, then fires `onContextLoss`, and `termwrap.ts` switches that terminal to the DOM renderer. So an evicted terminal is still blank for about 3 s before it renders through the DOM. The page no longer reloads. Option 2 removes even those 3 s by not hitting the cap.

## 6. Evidence

- `~/.agentmux/dev/camper-term-resize-no-blank-frame/07d4a10287c0c856/logs/cef-debug.log`, lines around 649–1734: the warnings, the recovery messages and the DOM fallbacks.
- Live page over CDP after the event: `.xterm` elements 21; with a WebGL canvas 16; DOM-rendered 5.
- Code: `frontend/bootstrap.ts` (`webglcontextlost` handler, `CONTEXT_LOSS_MAX_RELOADS = 3`, 60 s counter reset) and `frontend/app/view/term/termwrap.ts` `loadRendererAddon` (one `WebglAddon` per terminal, `onContextLoss` → DOM).
