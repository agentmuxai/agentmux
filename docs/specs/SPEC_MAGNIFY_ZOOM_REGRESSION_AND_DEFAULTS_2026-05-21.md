# SPEC: Magnify zoom regression + magnified-pane defaults

**Date:** 2026-05-21
**Author:** AgentX
**Status:** Draft — investigation complete, implementation pending

---

## 1. Summary

Three related changes to the pane *magnify* feature (the maximize button in
the pane header, top-right):

1. **Bug fix** — after a pane is magnified then restored, that pane is left
   in a broken state. Two failure modes, one root cause:
   - **Terminal / agent / swarm panes** — per-pane zoom (`Ctrl +/-` and
     `Ctrl+scroll`) stops working *for that pane*; the pane otherwise works.
   - **Browser panes** — far worse: the pane goes **black and stuck** (the
     native browser window is destroyed and never recreated).
   Panes that were never magnified are unaffected.
2. **Default change** — a magnified pane currently leaves a margin gap; it
   should cover the entire window.
3. **Default change** — a magnified pane is currently slightly translucent;
   it should be fully opaque (100%).

Items 2 and 3 are setting-default changes only. Item 1 is a real defect.
The architectural backdrop — including the reducer system and why browser
panes need special treatment — is in the companion
`SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md`.

---

## 2. Bug: zoom dies on a magnified-then-restored pane

### 2.1 Symptom

Magnify any pane via its header button, then restore it. After restore,
`Ctrl +`, `Ctrl -`, and `Ctrl+scroll` no longer zoom that pane. Reproduces
100% of the time, with any pane. Other panes still zoom normally.

### 2.2 Root cause — duplicate `Block` render sharing one registry entry

The magnify feature renders the magnified pane **twice**:

- `frontend/layout/lib/TileLayout.win32.tsx:502` — the original tile node
  stays mounted and merely gets the `tile-hidden` CSS class while magnified
  (`class={clsx("tile-node", { "tile-hidden": isMagnified() })}`).
- `frontend/layout/lib/TileLayout.win32.tsx:260` `MagnifiedPaneOverlay` —
  renders the *same node* a second time via
  `props.layoutModel.renderContent(nodeModel)` (line 300), into a separate
  overlay container "outside display-container to avoid stacking context
  issues".

So while magnified there are **two live `Block` components for the same
`blockId`**.

`Block` (`frontend/app/block/block.tsx:290`) registers itself in a
**`blockId`-keyed map**:

- `createEffect` (line 302) → `registerBlockComponentModel(blockId, { viewModel })`
- `onCleanup` (line 314) → `unregisterBlockComponentModel(blockId)` **and**
  `viewModel()?.dispose()`

`blockComponentModelMap` is keyed by `blockId` alone
(`frontend/app/store/global.ts:680-687`), so the two `Block` instances share
**one** map entry.

Lifecycle of the shared entry:

| Step | Event | `blockComponentModelMap[blockId]` |
|------|-------|-----------------------------------|
| 1 | Tile `Block` mounts | `{ viewModel: vm1 }` |
| 2 | Magnify → overlay `Block` mounts; its effect finds `vm1`, reuses it (no re-register) | `{ viewModel: vm1 }` |
| 3 | Restore → overlay `Block` unmounts → `onCleanup`: `unregisterBlockComponentModel(blockId)` **deletes the entry** and `viewModel().dispose()` **disposes `vm1`** | *(deleted)* |
| 4 | Tile `Block` is still mounted but never re-runs its effect (view type unchanged) → never re-registers | *(still deleted)* |

After restore the registry has **no entry** for that `blockId`, and the
shared view model `vm1` has been disposed. The tile `Block` keeps rendering
visually (it still holds `vm1` in its local signal), which is why the pane
*looks* fine — but anything that resolves the pane *through the registry* is
now broken.

### 2.3 Why this kills zoom specifically

Both zoom paths resolve the pane through `getBlockComponentModel`:

- Keyboard: `keymodel.ts:655` `Ctrl:=` → `zoomIn()` →
  `zoom.win32.ts:109` → `getBlockZoom()` →
  `getBlockComponentModel(blockId)` → `zoom.win32.ts:59`.
- Wheel: `app.tsx:268` `Ctrl+scroll` → `zoomBlockIn(blockId)` →
  `getBlockZoom()` → same lookup.

`getBlockZoom` (`zoom.win32.ts:58`):

```ts
const bcm = getBlockComponentModel(blockId);
if (!bcm?.viewModel) return null;        // ← registry entry gone → null
```

`zoomIn` / `zoomBlockIn` early-return when `getBlockZoom` returns `null`, so
both zoom gestures become silent no-ops. The pane keeps working otherwise
because it renders off the still-held (though disposed) `vm1`, not the
registry.

This also explains why only the magnified-then-restored pane is affected:
its registry entry is the only one that got deleted.

### 2.4 Browser panes — the same defect, but destructive

A browser pane is a native `CefBrowserView` child window, not DOM. Its DOM
side (`browser-view.tsx`) renders only a `.browser-placeholder`; the native
pane is created on mount (`browser_pane_create`), tracked to the placeholder
rect via `browser_pane_resize` (a `ResizeObserver` + a 200 ms interval), and
**closed on unmount (`browser_pane_close`)**. Lifecycle is coupled 1:1 to the
`BrowserViewComponent` mount/unmount cycle.

Apply the §2.2 duplicate render to that:

1. **Magnify** → the overlay mounts a *second* `BrowserViewComponent` for the
   same `block_id`. The host reducer's `TryRegisterBrowserPaneLive` returns
   `AlreadyLive`, so no second native window is created — but now **two
   `.browser-placeholder` nodes drive geometry**: `tile-hidden` is
   `visibility: hidden` (the node keeps its layout box), so the still-mounted
   tile placeholder reports the *tile* rect while the overlay placeholder
   reports the *magnified* rect. Their two 200 ms intervals fire
   `browser_pane_resize` alternately → the native HWND is yanked between the
   tile rect and the magnified rect.
2. **Restore** → the overlay `BrowserViewComponent` unmounts → its
   `onCleanup` fires `browser_pane_close` → the host reducer flips the
   *shared* `block_id` entry `Live → Closing` and **destroys the native
   window**.
3. The tile `BrowserViewComponent` is still mounted, still believes
   `paneCreated === true`. `createPane` runs only once (in `onMount`) so the
   pane is **never recreated**. Its resize calls hit a `Closing`/destroyed
   pane and are no-op'd by the reducer.

→ The browser pane is **black** (native window destroyed, no compositor) and
**stuck** (no recreate path). Same root cause as the zoom bug — the
duplicate `Block` render — but for browser panes it destroys the pane
because the native-window lifecycle is mount-coupled.

### 2.5 Fix options

**Option A (recommended) — render the magnified pane as a single instance.**
Do not mount a second `Block`. Instead, render the magnified node exactly
once and *reposition* it into the overlay (e.g. SolidJS `<Portal>` the
existing tile `DisplayNode`'s content into the overlay container, or move the
DOM node). One `Block` → one registry entry → no unregister-on-restore.
`tile-hidden` becomes unnecessary for the magnified node. This removes the
whole class of duplicate-render bugs but touches the magnify render path in
all three `TileLayout.*.tsx` files.

**Option B (terminal-only mitigation) — make registration duplicate-safe.**
Give each `Block` mount an identity token. `unregisterBlockComponentModel`
deletes the entry only if the caller is the current owner; the surviving
duplicate re-registers itself when it becomes the sole instance; `dispose()`
runs only when the last instance unmounts. This fixes the *zoom* symptom for
terminal/agent/swarm panes — but it does **not** fix browser panes (§2.4):
the two `.browser-placeholder` nodes still fight over `browser_pane_resize`
while magnified, and the mount-coupled `browser_pane_close` can still destroy
the native window. Option B is insufficient.

**Recommendation: Option A — and it is mandatory, not preferred.** The
duplicate render is the underlying defect. Only a single render instance
gives one registry entry, one `.browser-placeholder`, one
`browser_pane_create`/`close`. Option B papers over the zoom symptom and
leaves browser panes broken. See the companion architecture spec §4.A / §4.A′
for the single-instance design and the browser-pane reducer guarantees.

### 2.6 Acceptance criteria

- **Terminal/agent/swarm pane:** magnify, restore, then `Ctrl +`, `Ctrl -`,
  `Ctrl+scroll` all zoom that pane. Repeat several times — zoom keeps
  working. The shared view model is not disposed while the pane is on screen.
- **Browser pane:** magnify, restore — the pane stays live (not black), keeps
  its URL/scroll, and tracks the tile rect again. Repeat several times.
- While a browser pane is magnified, only one rect drives
  `browser_pane_resize` (no 0×0 / magnified-rect flicker).
- Regression tests: after a simulated magnify→restore cycle,
  `getBlockComponentModel(blockId)` still returns a live model; the host
  `browser_panes` entry for that `block_id` is still `Live` (never
  transitioned to `Closing`).

---

## 3. Default change: magnified pane covers the whole window

### 3.1 Current behaviour

`MagnifiedPaneOverlay` sizes the magnified pane from the
`window:magnifiedblocksize` setting, **defaulting to `0.9`**:

- `TileLayout.win32.tsx:263` — `magnifiedBlockSizeAtom() ?? 0.9`
- `TileLayout.linux.tsx:234`, `TileLayout.darwin.tsx:232` — same.

`containerStyle` (`TileLayout.win32.tsx:282`) then centres it:

```ts
const size = magnifiedNodeSize();          // 0.9
const margin = ((1 - size) / 2) * 100;     // 5
// top/left = 5%, width/height = 90%
```

→ a 5% margin on every side. That is the gap the user sees.

### 3.2 Change

Change the effective default of `window:magnifiedblocksize` from `0.9` to
`1.0` in all three `TileLayout.*.tsx` files (`?? 0.9` → `?? 1.0`). At `1.0`,
`margin` computes to `0` and the pane fills the overlay container.

The setting itself is retained — a user can still set a smaller value. Only
the default changes. `window:magnifiedblocksize` is `Option<f64>` in the
backend (`agentmux-srv/src/backend/wconfig/types.rs:167`, default `None`), so
the frontend fallback *is* the default; no backend change is required.

---

## 4. Default change: magnified pane opacity 100%

### 4.1 Current behaviour

A magnified pane renders translucent: `--block-bg-color` is translucent in
every theme (`theme.scss` default `rgba(0,0,0,0.5)` — 50%; named themes
~70%), and a magnified pane has **no opacity override** — so the blurred
magnify backdrop bleeds through it.

The `--magnified-block-opacity` CSS var *looks* like the control for this,
but it is misapplied: declared in `block.scss:308` and consumed only by the
`&.ephemeral` selector (`block.scss:312`) — never by `.magnified`. So
neither the var's default nor the `window:magnifiedblockopacity` setting
(`blockframe.tsx:754` feeds the var inline) has ever affected magnified
panes. There is no `.magnified` opacity rule anywhere in `block.scss`.

### 4.2 Change

A magnified pane is made **fully opaque, unconditionally** — two solid
layers, neither depending on a CSS variable resolving:

1. **Magnify overlay base.** `tilelayout.scss` `.magnify-pane` gets
   `background-color: rgb(from var(--block-bg-color) r g b)` — the theme
   colour with no alpha (fully opaque). This is the slot the single pane
   instance is reparented into, so it backs the whole magnified area.
2. **Block inner.** `block.scss` `&.magnified .block-frame-default-inner`
   gets the same opaque `rgb(from var(--block-bg-color) r g b)`, so the
   pane's own frame is opaque too (headers, rounded corners, browser-pane
   edges).

`rgb(from <color> r g b)` with the alpha omitted yields a fully opaque
colour — so the result does not depend on `--magnified-block-opacity`
resolving (an earlier var-based attempt failed when the var was out of
scope, leaving the declaration invalid and dropped).

The `--magnified-block-opacity` variable / `window:magnifiedblockopacity`
setting were never actually wired to magnified panes (only `.ephemeral`),
and remain so — left untouched. A configurable magnified-pane opacity, if
ever wanted, is a separate feature; the requirement here is simply 100%.

The blur (`--magnified-block-blur: 10px`,
`window:magnifiedblockblur*px`) is **out of scope** — left unchanged unless
the user asks.

---

## 5. Files touched (implementation checklist)

| Concern | File | Change |
|--------|------|--------|
| Magnify bug (Option A) | `frontend/layout/lib/TileLayout.{win32,linux,darwin}.tsx` | Single-instance magnified render — no duplicate `Block` mount (see arch spec §4.A) |
| Browser-pane robustness | `frontend/app/view/browser/browser-view.tsx`; verify host `browser_panes` reducer | One `.browser-placeholder` drives geometry; no `browser_pane_close` on a magnify transition; optional recreate-if-`Closed` recovery (arch spec §4.A′) |
| Regression tests | `frontend/layout/tests/`, browser-pane tests | Registry survives magnify→restore; host `browser_panes` entry stays `Live` |
| Cover whole window | `frontend/layout/lib/TileLayout.win32.tsx:263`, `TileLayout.linux.tsx:234`, `TileLayout.darwin.tsx:232` | `?? 0.9` → `?? 1.0` |
| Opacity 100% | `frontend/app/block/block.scss:308` | `--magnified-block-opacity: 0.95` → `1` |

The defaults (size, opacity) are frontend-only. The magnify/browser-pane fix
is frontend-led but may need a host reducer change if magnify becomes a
reducer-tracked relocate (arch spec §6). Per `CLAUDE.md`, feature PRs use the
changesets workflow — add a `task changeset -- patch "..."` entry; do **not**
bump `package.json`/`Cargo.toml`.

---

## 6. Open questions

- Option A is mandatory (browser panes rule out Option B) — confirm the
  single-instance render approach (Portal/reparent vs reducer-tracked
  relocate) per arch spec §6 before implementation.
- Should `window:magnifiedblocksize` continue to support sub-1.0 values now
  that the default is full-window, or is the setting effectively retired?
- Blur on the magnified pane — keep the 10px default, or revisit alongside
  the opacity change?
