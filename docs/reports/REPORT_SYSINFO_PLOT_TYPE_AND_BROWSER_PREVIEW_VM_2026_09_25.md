# REPORT — Sysinfo plot type stuck on CPU; split browser pane stays black

**Date:** 2026-09-25
**Author:** Camper @ narko
**Trigger:** Repo owner, testing PR #3752 (Pane Tab contract Phase 1) in `task dev`:
(1) right-click a Sysinfo pane → pick "Mem" does nothing, the plot stays on CPU;
(2) "Split down" on a browser pane from the context menu → the new browser is
black (earlier, in another dev build: a browser tab "black, then glitchy, comes
and goes").
**Status:** active — findings below; fixes land as separate PRs in the order of §4.

---

## 1. Neither bug comes from PR #3752

Both reproduce in the v0.57.4 portable build (`main` at `f39d06da1`, no Phase 1),
confirmed by the repo owner. PR #3752 is unaffected.

## 2. Split browser pane stays black — root cause found

### 2.1 What the logs show

`task dev` host log, dev data dir `~/.agentmux/dev/camper-pane-tab-phase1/…/logs`,
split at 14:57:26–28 UTC:

- The native page for the new block (`6d2412e`) is created and finishes loading
  (`pane-trace … event=load-end` at 14:57:28.936). Nothing is destroyed.
- **Two `BrowserViewModel`s are constructed for the same block**, 25 ms apart
  (`vm=rbqq7t`, `vm=p08eas`). Every browser block in the session shows the same
  pair.
- Both receive every IPC event, but only the second applies them: `p08eas` logs
  `state-write key=loading value=false` at 14:57:29.323. `rbqq7t` logs no
  `state-write` after construction, so its `loadingAtom()` stays `true`.
- The view holding `rbqq7t` keeps the first-load brain overlay mounted forever.
  That overlay carries `data-pane-overlay`, so `pane-overlay-auto.ts` keeps a
  full-pane hole cut through the native page (`[pane-airspace] … overlay_count: 1`
  at 14:57:28.474, never cleared). The pane shows the DOM behind it: the spinner,
  or dark background once it fades → "black".

### 2.2 Why there are two view models

`TileLayout.core.tsx:591` renders `previewElement()` — the drag-preview thumbnail,
`tabcontent.tsx`'s `renderPreview` → `<Block preview>` — for **every tile, always**,
not only while dragging. `block.tsx`'s preview branch (~L363) builds a private
ViewModel for it, on purpose (see that comment: sharing the real mount's vm caused
double headers, #3483).

That is safe only for view models without global side effects. `BrowserViewModel`'s
**constructor** has them:

1. `bpRegisterPane(blockId, projections)` — the browser-pane state store is keyed
   by block id alone, so the preview's registration **replaces** the real pane's
   projections. The real vm stops receiving state writes.
2. It dispatches `OpenTab` + `Navigate` on that shared slot, resetting the real
   pane's state.
3. On preview unmount, `dispose()` calls `bpUnregisterPane(blockId)` — **deleting
   the real pane's slot**. The live pane then drops every event
   (`post-close-event-dropped`) and reads `closed = true`
   (`use-pane-rect-sync.ts` stops syncing its rect).

Which vm ends up owning the slot depends on mount order, which changes whenever
the layout tree is rebuilt (a split, a tab move). That fits both the black split
pane and the earlier intermittent "comes and goes".

### 2.3 Proposed fix

A preview mount gets a side-effect-free view model: no slot registration, no IPC
subscriptions, no navigate, no unregister on dispose — just title/favicon for the
thumbnail. Test-first: (a) mounting and unmounting a preview vm leaves the real
pane's slot and projections untouched; (b) split-like remount ordering (preview
before/after real, preview unmount last) keeps the real pane receiving
`loading=false`. Audit the other block-id-keyed stores for the same constructor
pattern (agent document/layout/state stores, editor store).

## 3. Sysinfo plot type stuck on CPU — root cause found, fixed

### 3.1 What the live logs show

Temporary `[sysinfo-diag]` logging, one click on "Mem" (block `c1a544a`):

- 15:11:12.985 — the click handler runs (`body-click plotType=Mem`).
- 15:11:12.997 — `SetMetaCommand` resolves without error.
- The block's `plotTypeSelected` memo **never re-runs** afterwards; it stays on
  `"CPU + Mem"`. The repo owner's account matches: the first change (CPU →
  CPU + Mem) worked, every later one did nothing.

The server side is fine: `setmeta` merges the meta and broadcasts
`waveobj:update` (`server/mod.rs` `broadcast_meta_update`).

### 3.2 Root cause

`block.tsx`'s `makeViewModel` called `new ctor(...)` **inside `Block`'s
`createEffect`**. So:

1. every memo/effect a ViewModel constructor creates is owned by that effect
   run, and
2. every signal the constructor reads while building (Sysinfo:
   `loadInitialData` → `numPoints()` → block meta + full config) subscribes the
   effect to it.

The first meta change therefore re-runs the effect, which disposes the previous
run — and with it every memo of the ViewModel. The ViewModel object itself is
cached in the block-component registry and reused, so it lives on with dead memos
frozen at their last value. `BrowserViewModel` and `EditorViewModel` already
guard against this with their own `createRoot` (see `browser-model.ts`'s
constructor comment, the 2026-05-18 favicon regression); the other view models
with constructor memos/effects (sysinfo, term, launcher, armory, media, warden,
bundle, identity-pane, drone, swarm, and the default view model) did not.

### 3.3 Fix

`makeViewModel` builds every ViewModel inside its own `createRoot`: the
constructor gets an owner that lives as long as the ViewModel, and runs untracked
(Solid's `createRoot` sets `Listener = null`), so its reads no longer subscribe
the effect. The root inherits context from the calling owner and is not disposed
with it; `disposeViewModel` disposes it together with `vm.dispose()`, at the one
place `Block` disposes a ViewModel it created. Tests in `block.test.tsx`
("a ViewModel's own reactive state outlives meta changes"): a constructor memo
follows two consecutive meta changes (failed before the fix, stuck on the first
value), and unmounting the creating mount disposes the ViewModel's root.

### 3.4 Ruled out along the way

Ruled out by reading the code:

- The context menu path (`contextmenu.ts` → `cef-api.ts` `showJsContextMenu`)
  handles `radio` items like any other: a click removes the overlay and calls the
  registered handler.
- `SysinfoViewModel.getBodyContextMenuItems` / `getSettingsMenuItems` call
  `SetMetaCommand` with `sysinfo:type`, and the chart re-derives from it
  (`plotTypeSelectedAtom` → `metrics` → `sysinfo-view.tsx`'s `yvals`).
- `cpuplot` and `sysinfo` share the same ViewModel and view component.

The `[menu-guard] … behind native pane` errors in the host log are unrelated
(the menu still received the click).

## 4. Order of work (repo owner, 2026-09-25)

1. Sysinfo plot type (§3).
2. Black split browser pane (§2).
3. Pane Tab contract phases after Phase 1 (#3752).
