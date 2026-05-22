# SPEC: Magnify & Zoom — Implementation Plan

**Date:** 2026-05-21
**Author:** AgentX
**Status:** Draft — implementation plan, ready for review
**Companions:**
- `SPEC_MAGNIFY_ZOOM_REGRESSION_AND_DEFAULTS_2026-05-21.md` — the defects + default changes (the *what*).
- `SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md` — architecture analysis (the *why*).

This document is the *how*: a phased, file-level implementation plan.

---

## 1. Overview

Four phases, ordered so each lands independently and de-risks the next.
Phase 0 ships the user-visible default changes immediately; Phases 1–2 are
mechanical de-duplication that shrink the surface; Phase 3 is the real fix
for the magnify zoom regression and the browser-pane black/stuck failure;
Phase 4 is optional cleanup.

| Phase | Goal | Risk | Blocks |
|-------|------|------|--------|
| 0 | Magnified-pane defaults (full-window, 100% opacity) | Low | — |
| 1 | Collapse the `zoom.*` platform split | Low | — |
| 2 | Extract a shared `TileLayout` core | Medium | 1 |
| 3 | Single-instance magnified render + browser-pane robustness | High | 2 |
| 4 | Consolidate the zoom module | Low | 1 |

Each phase = one PR = one changeset. Phases 0/1/4 can land in parallel;
3 should land on top of 2 so the fix is written once, not pasted ×3.

---

## 2. Guiding principles

1. **One render per pane.** A pane is one `Block` component instance.
   "Magnified" is a position/size *state* of that instance, never a second
   instance.
2. **Native panes are reducer-owned.** Browser-pane create/resize/close flow
   through the host reducer (`HostState.browser_panes`). The DOM never races
   the reducer for geometry or lifecycle.
3. **No behavioural change without a test.** Every phase ships regression
   tests; Phase 3 ships the magnify→restore tests in the companion spec §2.6.
4. **De-duplicate before fixing.** The magnify fix is written once, in shared
   code — never pasted into three `TileLayout.*.tsx` files.

---

## 3. Phase 0 — Magnified-pane defaults

**Goal:** magnified pane covers the whole window and is fully opaque.
Independent of the refactor; ship first.

### 3.1 Changes

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.win32.tsx:263` | `magnifiedBlockSizeAtom() ?? 0.9` → `?? 1.0` |
| `frontend/layout/lib/TileLayout.linux.tsx:234` | same |
| `frontend/layout/lib/TileLayout.darwin.tsx:232` | same |
| `frontend/app/block/block.scss:308` | `--magnified-block-opacity: 0.95` → `1` |

`window:magnifiedblocksize` / `window:magnifiedblockopacity` stay
`Option<f64>` (default `None`) in `agentmux-srv/.../wconfig/types.rs` — the
frontend fallbacks *are* the defaults, so no backend change.

> Note: the three `?? 0.9` edits collapse to **one** site after Phase 2. If
> Phase 2 lands first, Phase 0's size change is a single-file edit. Ordering
> 0-before-2 is fine too — just three identical edits.

### 3.2 Acceptance

- Magnify any pane → it fills the window edge-to-edge, no margin gap.
- Magnified pane background is fully opaque.
- `window:magnifiedblocksize` / `window:magnifiedblockopacity` still
  override when a user sets them.

### 3.3 Tests

- Unit: `containerStyle` at size `1.0` yields `margin = 0`,
  `width/height = 100%`.

---

## 4. Phase 1 — Collapse the `zoom.*` platform split

**Goal:** delete ~370 lines of byte-identical duplication. The three
`zoom.{win32,linux,darwin}.ts` files differ only in header comments
(verified by `diff`); all executable code is identical. Platform behaviour
lives entirely in `window-header.{scss,darwin.scss}`.

### 4.1 Changes

- Rename `frontend/app/store/zoom.win32.ts` → `zoom.ts` (canonical content).
- Delete `zoom.linux.ts`, `zoom.darwin.ts`, `zoom.platform.ts`.
- Update importers of `@/store/zoom.platform` (`app.tsx`, `keymodel.ts`,
  any others) to import `@/store/zoom`.
- Fold the platform-specific *comments* that were in each variant into one
  block comment in `zoom.ts` so the WebKitGTK / macOS notes are not lost.

### 4.2 Pre-check (do this first)

Confirm Vite's `platformResolve` plugin tolerates a plain, non-suffixed
module — i.e. a `zoom.ts` with no `.platform`/`.win32` sibling resolves
normally. If the plugin *requires* the suffix pattern for any path it has
seen before, either (a) keep a one-line `zoom.platform.ts` re-exporting
`./zoom`, or (b) patch the plugin. Decide before writing the deletion.

### 4.3 Acceptance

- `task build:frontend` succeeds on all three platforms.
- Zoom (keyboard + wheel, pane + chrome) behaves identically to before on
  Windows; smoke-check Linux/macOS if available.

### 4.4 Tests

- Existing zoom tests pass against the single module.

---

## 5. Phase 2 — Extract a shared `TileLayout` core

**Goal:** stop the magnify fix from being a three-file paste.
`TileLayout.{win32,linux,darwin}.tsx` are ~730–840 lines, ~75–90 % shared.
The magnify-relevant pieces — `MagnifiedPaneOverlay`, `DisplayNode`'s
magnify logic, `DisplayNodesWrapper` — are copy-pasted verbatim ×3.

### 5.1 Changes

- New `frontend/layout/lib/TileLayout.shared.tsx` containing the
  platform-identical components/helpers. Start with the magnify surface
  (`MagnifiedPaneOverlay`, `DisplayNodesWrapper`, and the magnify branch of
  `DisplayNode`); migrate more later as a follow-up.
- `TileLayout.{win32,linux,darwin}.tsx` import from `TileLayout.shared.tsx`
  and keep **only** genuine platform deltas: Win32 launcher wiring, Win32
  drag-preview image generation, the double-click-maximize target wiring,
  and any confirmed platform-specific markup.

### 5.2 Method (avoid silent platform regressions)

1. `diff` win32↔linux and linux↔darwin. Classify every differing line as
   *intentional* (launcher, drag-preview, dblclick) or *drift* (comments,
   formatting, stale copies).
2. Move only lines that are byte-identical across all three into
   `TileLayout.shared.tsx`.
3. For intentionally-divergent pieces, expose a prop / injection point on
   the shared component rather than forking it.
4. Land Phase 2 as a **pure refactor** — no behavioural change, existing
   layout tests must pass untouched.

### 5.3 Acceptance

- All three `TileLayout.*.tsx` build; layout/tile tests pass unchanged.
- No visual or behavioural diff in tiling, dragging, splitting, magnify.

### 5.4 Tests

- Existing `frontend/layout/tests/layoutModel.test.ts` and drag/tile tests
  pass with zero edits.

---

## 6. Phase 3 — Single-instance magnified render + browser-pane robustness

**Goal:** fix the magnify zoom regression *and* the browser-pane
black/stuck failure, at the root, in shared code. This is the core phase.

### 6.1 Root cause recap

Today the magnified pane is rendered **twice**: `DisplayNode` keeps the
tile `<Block>` mounted (`tile-hidden` = `visibility:hidden`) and
`MagnifiedPaneOverlay` calls `renderContent` to mount a **second** `<Block>`
for the same `blockId`. Two `Block`s share one `blockId`-keyed registry
slot and (for browser panes) one native window. The overlay copy's
`onCleanup` on restore deletes the registry slot / disposes the view model
/ fires `browser_pane_close`. See companion spec §2.

### 6.2 Design — reparent one instance, do not re-render

Render the pane's `<Block>` **once** and *move its DOM node* between the
tile slot and the magnify overlay slot. Moving a DOM subtree with
`appendChild` does **not** dispose a SolidJS component (its lifecycle is
tied to the reactive owner, not DOM position) — so the `Block`, its
`ViewModel`, its registry entry, and (critically) the single
`.browser-placeholder` and the native browser window all survive the
magnify/restore transition untouched.

Concretely:

1. **`MagnifiedPaneOverlay` stops rendering content.** It becomes a
   positioned, styled container only (backdrop + `.magnify-container` with
   `containerStyle`). On mount it publishes its inner mount element to the
   layout model, e.g. `layoutModel.setMagnifyMount(el)` /
   `layoutModel.magnifyMountAtom()`. Remove the `renderContent(nodeModel)`
   call (`TileLayout.win32.tsx:300` and platform twins / shared).

2. **`DisplayNode` owns a stable content wrapper.** `leafContent()` renders
   into a wrapper `<div class="tile-leaf">` that is created once. The
   `<Block>` lives inside it, unconditionally, for the node's whole life.

3. **Reparent on magnify state change.** An effect in `DisplayNode`:
   ```
   createEffect(() => {
     const mount = layoutModel.magnifyMountAtom();
     if (isMagnified() && mount) mount.appendChild(wrapperEl);
     else tileNodeRef.appendChild(wrapperEl);   // back to the tile
   });
   ```
   On restore the wrapper is reparented to the tile *before* the overlay
   `<Show>` tears down (the effect runs on the `isMagnified()` flip, which
   precedes the overlay unmount), so the wrapper is never destroyed with
   the overlay.

4. **Drop the `tile-hidden` twin.** With a single instance there is no
   hidden duplicate; `tile-hidden` / the `.magnify-pane` content slot are
   removed. The empty tile node may keep a placeholder box for layout
   stability while magnified — confirm during implementation.

### 6.3 Browser-pane specifics (companion arch spec §4.A′)

Single-instance render already gives browser panes **one**
`.browser-placeholder`, **one** `browser_pane_create`, **one**
`browser_pane_close`, and **one** geometry authority — the reparent moves
that single placeholder, and its `ResizeObserver` + 200 ms interval then
report the magnified rect (and the tile rect again on restore). The
HWND-flicker and the destroy-on-restore both disappear.

Two added guarantees:

- **No close on a magnify transition.** Verify the reparent keeps the
  `BrowserViewComponent` subtree mounted (no unmount → no `onCleanup` →
  no `browser_pane_close`). Add an explicit test asserting the host
  `browser_panes` entry stays `Live` across magnify→restore.
- **Defensive recovery.** In `browser-view.tsx`, if the component is
  mounted with `paneCreated() === true` but a host query reports the
  `block_id` is `Closed`/absent, call `createPane` again. This uses the
  host reducer's existing three-way `TryRegisterBrowserPaneLive`
  (`Fresh`/`AlreadyLive`/`Closing`) signal and guarantees a pane can never
  be permanently black even if a future change reintroduces a race.

### 6.4 Files

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.shared.tsx` | `MagnifiedPaneOverlay` → container-only; publishes mount node. `DisplayNode` → stable wrapper + reparent effect; remove `tile-hidden` twin |
| `frontend/layout/lib/layoutModel.ts` | Add `magnifyMount` atom + `setMagnifyMount` (mount-node handoff) |
| `frontend/layout/lib/tilelayout.scss` | Remove `.tile-node.tile-hidden` / `.magnify-pane`; keep `.magnify-container` |
| `frontend/app/view/browser/browser-view.tsx` | Defensive recreate-if-`Closed`; confirm no close on reparent |
| `frontend/layout/lib/TileLayout.{win32,linux,darwin}.tsx` | Drop now-shared magnify code (consumed from shared) |

No host Rust change is expected — the reducer already de-dupes create and
the single-instance render removes the spurious close. *If* validation shows
the reparent cannot keep the subtree mounted on some platform, escalate to
the arch-spec §6 fallback (a reducer-tracked `relocate` that keeps the entry
`Live`); that *would* add a host reducer command.

### 6.5 Acceptance (companion spec §2.6)

- **Terminal/agent/swarm pane:** magnify → restore → `Ctrl +`, `Ctrl -`,
  `Ctrl+scroll` all zoom that pane. Repeat ×5. View model never disposed
  while on screen.
- **Browser pane:** magnify → restore → pane stays live (not black), keeps
  URL/scroll, tracks the tile rect again. Repeat ×5.
- While magnified, exactly one rect drives `browser_pane_resize` (no
  flicker).
- Magnify still renders above other panes (z-order correct).

### 6.6 Tests

- `getBlockComponentModel(blockId)` returns a live model after a simulated
  magnify→restore cycle.
- Host `browser_panes[block_id]` stays `Live` across magnify→restore (Rust
  reducer test, alongside the existing lifecycle tests in `browser_panes.rs`).
- `browser-view.tsx`: recreate path fires when host reports `Closed`.

### 6.7 Risks

- **Subtree-preserving reparent.** The whole fix depends on `appendChild`
  moving the `<Block>` DOM without SolidJS disposing it. Prototype this in
  isolation first; if it disposes, use the §6 fallback.
- **Non-magnified browser panes during magnify.** Native browser HWNDs
  paint above DOM, so a browser pane in another tile could show above a
  magnified DOM pane. Verify the host hides/occludes non-magnified browser
  panes while something is magnified — this may be pre-existing behaviour;
  confirm, file a follow-up if not.
- **Effect ordering.** The reparent effect must see `magnifyMountAtom()`
  populated before it runs on magnify-on. Guard for `mount == null`.

---

## 7. Phase 4 — Consolidate the zoom module (optional follow-up)

**Goal:** one zoom module, one dispatch path. Currently zoom has four
entrypoints (keyboard via `keymodel`, wheel via an ad-hoc `window` listener
in `app.tsx`) and two domains (pane vs chrome) interleaved.

### 7.1 Changes

- `zoom.ts` exposes a small, explicit surface: `paneZoom.{in,out,reset}`,
  `chromeZoom.{in,out,reset}`, and one pane-resolution helper.
- Route the wheel path (`AppZoomHandler`) and the keyboard path
  (`keymodel.ts`) through that surface so clamps, steps, and the zoom
  indicator live in one place.

Lower priority; do after Phase 3 stabilises. Not required for the bug fix.

---

## 8. Testing strategy (cross-cutting)

- **Unit** — `containerStyle` (Phase 0); zoom module (Phase 1/4); reparent
  registry survival (Phase 3).
- **Rust reducer** — `browser_panes` stays `Live` across magnify→restore
  (Phase 3), beside the existing Live→Closing lifecycle tests.
- **Manual matrix** — for Phase 3, the magnify→restore cycle ×5 on each pane
  type: terminal, agent, swarm, browser, editor. Browser is the critical
  case; editor is DOM-rendered (CodeMirror) so behaves like terminal.
- **Platform** — Phase 1/2 need a build check on all three platforms;
  Phase 3 behaviour is platform-shared after Phase 2 but still smoke-test
  Win32 (native HWND path) primarily.

---

## 9. PR & changeset plan

Per `CLAUDE.md`: feature PRs use the changesets workflow
(`task changeset -- patch "..."`); do **not** bump `package.json` /
`Cargo.toml`.

| PR | Phase | Changeset summary |
|----|-------|-------------------|
| 1 | 0 | `fix(magnify): magnified pane fills the window and is fully opaque` |
| 2 | 1 | `refactor(zoom): collapse the identical zoom platform split into one module` |
| 3 | 2 | `refactor(layout): extract shared TileLayout core` |
| 4 | 3 | `fix(magnify): single-instance magnified render — fixes zoom + browser-pane black/stuck` |
| 5 | 4 | `refactor(zoom): consolidate zoom entrypoints` (optional) |

PRs 1, 2, 5 are independent and can land in any order. PR 3 precedes PR 4.
Each goes through the normal review path (reagent + codex).

---

## 10. Rollback & sequencing notes

- Phases 0/1/2/4 are low-risk and individually revertable.
- Phase 3 is the behavioural change; if a regression surfaces, reverting
  PR 4 restores the (buggy but known) duplicate-render behaviour without
  touching Phases 0–2.
- If Phase 2 slips, Phase 3 can still ship by pasting the fix into the three
  `TileLayout.*.tsx` files — explicitly discouraged, but unblocks the fix.

---

## 11. Out of scope

- Window-maximize focus loss after `SW_RESTORE` (arch spec P5) — separate
  follow-up; add a `window-state-change` event then.
- The `useWindowDrag.*` platform split — fold into Phase 2 only if cheap;
  otherwise a later cleanup.
- Magnified-pane blur (`window:magnifiedblockblur*px`) — unchanged.
- Renaming "magnify" vs "maximize" terminology (arch spec §4.E) — user-facing
  string change; needs product sign-off, tracked separately.
