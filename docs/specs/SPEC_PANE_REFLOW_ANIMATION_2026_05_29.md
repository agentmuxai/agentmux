# SPEC: Coordinated Pane Reflow Animation (DOM + native browser panes)

- **Date:** 2026-05-29
- **Author:** AgentX
- **Status:** Design / implementation plan
- **Area:** `frontend/layout/`, `frontend/app/block/`, `frontend/app/view/browser/` (+ reuses existing `agentmux-cef` IPC; **no new Rust required**)
- **Supersedes the approach in:** PR #1156 (the `DefaultAnimationTimeS 0→0.15` one-liner — ineffective; see `docs/analysis/ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md`)
- **Goal:** When a pane opens/closes/splits/rebalances, **all** panes — DOM (terminal/agent/editor) **and** native browser panes — glide to their new geometry over ~150 ms instead of snapping.

---

## 1. Why the naive fix failed (recap)

The `.tile-node` wrapper transition works, but the visible pane **content** is sized from a **debounced inner rect** whose delay is `animationTimeS` (`block.tsx:166` → `layoutModelHooks.ts:103`). Raising `animationTimeS` just delays the content snap. And browser panes are **native windows** — CSS can't move them at all; they're positioned by `browser_pane_resize` → `SetWindowPos`.

So a real solution must (a) make DOM **content** animate with its wrapper, and (b) drive the **native** window every frame.

---

## 2. Architecture decision — one clock, native tracks DOM

**Chosen:** a single **frontend rAF interpolation loop** is the sole animation driver. Native panes are kept in sync **by construction**: each animation frame we read the browser pane's live (CSS-animating) placeholder rect and forward it to the host via the existing `browser_pane_resize` IPC. The native HWND therefore tracks exactly what the DOM is doing.

**Rejected:** an independent host-side (tokio) interpolation loop. It would run on a separate clock/easing from the DOM CSS transition → visible drift between a browser pane and its neighbors (the ugliest possible failure). The exploration confirmed tokio interval jitter (±2–5 ms) and that there is no existing host timer infra to reuse. Reusing `browser_pane_resize` needs zero new Rust and cannot drift.

### Current call paths (verified)
- **Native:** FE `invokeCommand("browser_pane_resize", {block_id,x,y,width,height})` *(device px)* → `ipc.rs` → `BrowserPaneManager::resize` → `SetWindowPos(hwnd, …, SWP_NOACTIVATE)` (`browser_panes.rs:223`). Thread-safe from the IPC tokio thread; ~1–2 ms.
- **Browser pane rect source:** `browser-view.tsx` `syncPosition()` reads `placeholderRef.getBoundingClientRect()` (×dpr) and sends it; today triggered by a `ResizeObserver` + 200 ms poll, deduped by `lastSentRect`.
- **DOM wrapper:** `DisplayNode` (`TileLayout.win32.tsx:336`) applies `setTransform()` (`utils.ts:63`) → `translate3d + width/height` to `.tile-node` **immediately**.
- **DOM content:** `block.tsx:166` sizes content from `useDebouncedNodeInnerRect` (debounced by `animationTimeS`).
- **Overlay clip** (`browser_panes_set_overlay_clip` → `SetWindowRgn`) is independent of pane position and is already rAF-coalesced (`pane-overlay.ts`).

---

## 3. Design

### 3.1 Single animation coordinator (the clock)
Add a small per-layout animation driver (in `layoutModel` or a dedicated hook used by `TileLayout`) that:
1. Detects an old→new geometry change for the tab's nodes (watch `additionalProps`/`transform` deltas).
2. Exposes an `isAnimating()` signal + the current animation start time and `animationTimeS` duration.
3. Runs a `requestAnimationFrame` loop for the duration; on each frame computes `progress = clamp(elapsed/duration)` and an eased value (cubic-bezier matching the CSS timing function — see §3.4).

This same signal/loop coordinates both the DOM-content sizing (§3.2) and the native-pane tracking (§3.3), so everything shares one clock and one easing.

### 3.2 DOM panes — content animates with the wrapper
- Enable the wrapper transition (`animationTimeS > 0`; the `.animate .tile-node` rule already transitions `width,height,transform`).
- Fix the **content** so it animates instead of snapping: on **open/close/split/rebalance** (i.e. *not* a resize drag), apply the new inner rect **immediately** and let the content size transition with the wrapper (add a CSS `transition: width,height` on the block content matching `--animation-time-s`). Keep the **debounce only for the resize-drag path** (`isResizing` already branches at `layoutModelHooks.ts:113`) so dragging a splitter doesn't reflow heavy content every frame.
- Net: terminal/agent/editor content glides with its wrapper.

### 3.3 Native browser panes — track the DOM per frame
- While `isAnimating()` is true, `browser-view.tsx` runs `syncPosition()` **every rAF** (not just on ResizeObserver/poll), reading the placeholder's live `getBoundingClientRect()` and sending `browser_pane_resize`. Because the placeholder is inside the CSS-animating `.tile-node`, its sampled rect *is* the animation curve → the native window follows the DOM exactly.
- Keep the existing `lastSentRect` dedupe (skips frames with no change). Stop the per-frame sampling when `isAnimating()` flips false; resume the ResizeObserver/200 ms safety net.
- The overlay-clip path already coalesces; if a pane moves under an overlay mid-animation, the existing clip refresh handles it. Verify no stale clip after the animation settles.

### 3.4 Easing & duration (best practice)
- Duration: **150 ms** (matches the file's reveal-gate + placeholder timings). Make it the single `animationTimeS` value.
- Easing: **ease-out** (`cubic-bezier(0.2, 0, 0, 1)` or similar) — fast start, gentle settle, the standard for open/close. Apply the *same* curve to the CSS transitions and the JS rAF interpolation so DOM and native match. (The current `.tile-node` rule uses `linear`; switch to ease-out.)

### 3.5 Reduced motion & correctness
- `prefers-reduced-motion` (or app setting via `prefersReducedMotionAtom`): **no animation** — apply final geometry immediately (current behavior). The coordinator must short-circuit (`isAnimating()` stays false), so DOM content applies instantly (existing `:113` branch) and the browser pane sends the final rect once.
- During an active **resize drag** (`isResizing`): no open/close animation; content stays debounced/instant as today.
- **Magnify**: unaffected (separate path; panes are `display:none`).
- Mid-animation **close**: if a pane closes during an animation, the browser pane's `Live` check fails host-side (`resize` no-ops) — safe. Cancel any rAF loop on unmount.

---

## 4. Implementation phases (each testable in `task dev`)

**Phase 1 — Animation clock + DOM wrapper.** Add the coordinator (`isAnimating`, rAF loop, eased progress); enable `animationTimeS`; switch `.tile-node` timing to ease-out. Verify wrapper glides on open/close.

**Phase 2 — DOM content sync.** In `layoutModelHooks.ts` / `block.tsx`: on open/close apply inner rect immediately + CSS-transition content width/height; keep debounce for drag. Verify terminal/agent/editor content glides (no snap). Watch xterm reflow cost.

**Phase 3 — Native browser-pane tracking.** In `browser-view.tsx`: drive `syncPosition()` from the coordinator's rAF while `isAnimating()`. Verify a browser pane glides in sync with neighbors on open/close/split.

**Phase 4 — Reduced motion, edge cases, polish, tests.** Reduced-motion snap; mid-animation close; multi-pane splits; overlay-clip settle; unit tests where feasible (`vitest`). Tune duration/easing.

---

## 5. Risks & mitigations
- **CEF relayout per frame (the main risk):** resizing a live browser pane re-lays-out the web page each `SetWindowPos`. For ~9 frames over 150 ms on a heavy page this could stutter. Mitigations: keep duration ≤150 ms; dedupe identical rects (already done); if it stutters, fall back to **animate position, snap size** for browser panes, or gate browser-pane animation behind a setting. Position-only moves are cheap; size changes are the costly part.
- **Main-thread starvation:** if JS jank stalls the rAF loop, DOM *and* native stall together (they stay in sync — acceptable, the whole frame just drops).
- **Per-frame IPC:** ~9–10 localhost POSTs per browser pane per animation; with 1–2 browser panes this is negligible. Dedupe avoids no-op frames.
- **Overlay-clip staleness:** confirm the clip is correct after the pane settles (it's rAF-coalesced; a final flush at animation end may be needed).

---

## 6. Files to touch
| File | Change |
|---|---|
| `frontend/layout/lib/layoutModel.ts` | Enable `animationTimeS` (0.15); expose/host the `isAnimating` clock |
| `frontend/layout/lib/layoutModelHooks.ts` | Open/close → immediate inner rect; debounce only on drag; feed coordinator |
| `frontend/layout/lib/TileLayout.win32.tsx` (+ darwin/linux) | Drive/observe the rAF coordinator; expose `isAnimating` to children |
| `frontend/layout/lib/tilelayout.scss` | `.tile-node` timing → ease-out; reduced-motion override (from PR #1156) |
| `frontend/app/block/block.tsx` | Transition content width/height; consume immediate rect on open/close |
| `frontend/app/view/browser/browser-view.tsx` | Per-frame `syncPosition()` while animating |
| *(no Rust changes)* | reuse `browser_pane_resize` / `SetWindowPos` |

---

## 7. Why this is the robust choice
- **No drift:** one clock (frontend rAF); native reads the DOM's real position each frame.
- **No new native surface:** reuses the existing, fast `browser_pane_resize`; nothing new to get wrong in the host.
- **Covers all pane types:** DOM panes via CSS+content-sync; browser panes via per-frame tracking.
- **Degrades safely:** reduced-motion snaps; drag path unchanged; heavy-page browser resize has a defined fallback.
