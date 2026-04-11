# PR #267 + #276 Takeover Report

**Date:** 2026-04-02
**Author:** AgentA (taking over from AgentX)

## PR #267: fix(win11): pane focus ring — DOM renderer + backdrop-filter

**Status:** Approved by reagent, needs splitting
**Branch:** `agentx/fix-win11-pane-focus-highlight`

### What it does (4 changes bundled together)

1. **Default to DOM renderer on Windows** (`term.tsx`, `termwrap.ts`)
   - WebGL canvases on Win11 get promoted to DWM hardware overlay planes
   - CSS borders can't paint above hardware overlays → focus ring invisible
   - DOM renderer not promoted → focus ring visible
   - Opt-in to WebGL via `term:disablewebgl=false`

2. **`backdrop-filter: blur(0.1px)` on `.block-mask`** (`block.scss`)
   - Forces compositor to render the mask above all sampled layers
   - Guarantees focus ring renders above xterm's scroll layer

3. **Remove `z-index` from `.block-focused`** (`block.scss`)
   - `z-index` creates stacking context that isolates xterm's GPU layer
   - Without it, `.block-mask` at `z-index:50` composites correctly

4. **Win11 `dragend` safety net** (`TileLayout.win32.tsx`)
   - Prevents `activeDrag` from sticking after snap-layout interruption
   - **Separate concern — should be its own PR**

### Concerns

- **DOM renderer default degrades terminal performance.** DOM is significantly slower for fast-scrolling output (e.g., `cat` large file, build logs). Most users won't know to opt-in to WebGL.
- **`backdrop-filter` hack is fragile** — relies on Chromium compositor behavior.
- **No version bump** — has bump commits but they conflict with current main.

### Plan: Split into 2 PRs

**PR A:** CSS focus ring fix (block.scss + blockframe.tsx)
- `backdrop-filter: blur(0.1px)` on `.block-mask`
- Remove `z-index` from `.block-focused`
- Move `BlockMask` to last in DOM
- Low risk, pure CSS

**PR B:** DOM renderer default on Windows (term.tsx + termwrap.ts)
- Needs perf testing before merging
- Consider: is `backdrop-filter` alone sufficient without switching renderers?

**PR C:** Win11 dragend safety net (TileLayout.win32.tsx)
- Separate concern, separate PR

---

## PR #276: fix(term): eliminate character echo delay

**Status:** Changes requested by reagent (hardcoded path in package-portable.ps1)
**Branch:** `agentx/fix-keyboard-echo-delay`

### What it does

- Small PTY writes (≤512 bytes) bypass `requestAnimationFrame` and write directly to xterm.js
- Eliminates 16-32ms latency on character echo during PTY output
- Removes noisy `console.log` from RAF write path

### Root cause

`scheduleRafWrite` (scroll-flicker fix) added RAF latency to ALL PTY output including single-byte echoes. When a write was in flight, new data waited for the full current write + another RAF cycle = 24ms+ delay per keystroke during output.

### Concerns

- **Has unrelated files:** `scripts/package-portable.ps1` with hardcoded user path, `CLAUDE.md` changes, bump commits
- **Core fix is sound:** 512-byte threshold is reasonable, xterm.js serializes writes internally

### Plan: Cherry-pick the termwrap.ts change only

The actual fix is a single file change in `termwrap.ts`. Everything else is noise from the other agent's session.

---

## Execution Plan

1. Close PR #267 and #276 (from agentx branch, too much noise)
2. Cherry-pick the good changes onto clean branches from main:
   - `agenta/focus-ring-css` — block.scss + blockframe.tsx changes
   - `agenta/dom-renderer-windows` — term.tsx + termwrap.ts renderer change (needs perf test)
   - `agenta/dragend-safety` — TileLayout.win32.tsx
   - `agenta/fix-echo-delay` — termwrap.ts RAF bypass
3. PR each separately with reagent review
