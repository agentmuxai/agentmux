# SPEC: Maximize & Zoom — Architecture Analysis

**Date:** 2026-05-21
**Author:** AgentX
**Status:** Draft — analysis for discussion
**Companion:** `SPEC_MAGNIFY_ZOOM_REGRESSION_AND_DEFAULTS_2026-05-21.md`
(the immediate bug fix + default changes; this doc is the architectural
backdrop that explains *why* that bug was possible).

---

## 1. Scope

Two feature clusters that overlap in user language and in code:

- **"Maximize"** — actually two unrelated mechanisms sharing one word.
- **"Zoom"** — one core behaviour reached through four entrypoints.

This document maps the current architecture, names the structural problems
(duplication, fragmentation, a registry that cannot represent the magnified
pane), and proposes a target architecture and sequencing.

---

## 2. Current architecture

### 2.1 "Maximize" is two different things

| | Window maximize | Pane magnify |
|---|---|---|
| Scope | OS top-level window | One pane inside the tab layout |
| Trigger | Title-bar button (`system-status.tsx`), double-click title bar (`useWindowDrag.*`), Command Palette (`command-registry.ts` `window:maximize`) | Pane-header button (`blockframe.tsx` `OptMagnifyButton`), `Escape`, backdrop click |
| Implementation | `agentmux-cef/src/commands/window.rs::maximize_window` → Win32 `ShowWindow(SW_MAXIMIZE/SW_RESTORE)` | `layoutModel.ts` + `layoutMagnify.ts` + `MagnifiedPaneOverlay` in `TileLayout.*.tsx` |
| State | OS-owned (`WINDOWPLACEMENT`) | `treeState.magnifiedNodeId` |
| Settings | — | `window:magnifiedblock{size,opacity,blurprimarypx,blursecondarypx}` |

**Naming collision.** The pane-header control is an `OptMagnifyButton`
(`title: "Magnify"`), but users — reasonably — call it "maximize", and the
icon set includes `window-maximize`. The reported "maximize breaks zoom" bug
was *pane magnify*, not window maximize. The codebase should pick one term
per mechanism and use it everywhere (UI, icons, code).

### 2.2 Zoom — four entrypoints, one core

```
keyboard  Ctrl +/-/0   keymodel.ts globalKeyMap ─┐
wheel     Ctrl+scroll  app.tsx AppZoomHandler ────┼─→ zoom.{os}.ts
                                                  │     ├─ per-pane zoom  (term:zoom block meta)
                                                  │     └─ chrome zoom    (--zoomfactor CSS var)
pane resolution ─────────────────────────────────┘
   getBlockComponentModel(blockId)  ← global.ts blockComponentModelMap
```

- **Per-pane zoom** scales a terminal/agent/swarm pane's font via the
  `term:zoom` block-meta value.
- **Chrome zoom** scales the title bar + status bar via a `--zoomfactor`
  CSS variable.
- Both keyboard and wheel paths resolve the target pane through
  `getBlockComponentModel(blockId)` — a `Map<blockId, model>` in
  `global.ts`.

### 2.3 Platform split

`zoom`, `TileLayout`, and `useWindowDrag` are each split into
`.win32` / `.linux` / `.darwin` files plus a `.platform` stub that Vite's
`platformResolve` plugin rewrites at build time.

### 2.4 Browser panes — a native window, not DOM

A browser pane is **not** rendered DOM. It is a `CefBrowserView` — a native
OS child window (HWND on Windows) that paints *above* the host webview. The
DOM side (`browser-view.tsx`) renders only a `.browser-placeholder` div; the
native pane is positioned to track that div:

- `onMount` → `browser_pane_create` (creates the native HWND, once).
- A `ResizeObserver` + a 200 ms `setInterval` call `syncPosition`, which
  reads `placeholderRef.getBoundingClientRect()` and fires
  `browser_pane_resize` to move/size the native HWND to match.
- `onCleanup` → `browser_pane_close` (flips the backend pane to `Closing`,
  destroys the HWND).

So a browser pane's **entire lifecycle is coupled to the
`BrowserViewComponent` mount/unmount cycle**, and its geometry is driven by a
single `.browser-placeholder` DOM node.

### 2.5 The reducer system

Browser-pane state is reducer-managed on **both** sides:

- **Host reducer** (`agentmux-cef/src/reducer/mod.rs`) — `HostState.browser_panes:
  HashMap<block_id, BrowserPaneEntry>` with an explicit `BrowserPaneLifecycle`
  (`Created/Live → Closing → Closed`). Commands: `TryRegisterBrowserPaneLive`
  (three-way `Fresh` / `AlreadyLive` / `Closing`), `CompleteBrowserPaneCreate`,
  `EnqueueBrowserPaneClose`. The reducer is the authority; `browser_panes.rs`
  is a thin executor. Late IPCs against a `Closing` pane are deliberately
  no-op'd.
- **Frontend reducer** (`frontend/app/store/browser-pane-state/`) — slice #9;
  a pane must be `registerPane`'d synchronously in the `BrowserViewModel`
  constructor.

The reducer keys panes by `block_id`. It correctly *protects against double
create* (a second `browser_pane_create` for a live `block_id` returns
`AlreadyLive`) — but it has **no concept of a pane being presented in two
surfaces at once**, and a `browser_pane_close` from *either* surface flips the
single shared entry to `Closing`. Any robustness fix must go *through* the
reducer, not around it.

---

## 3. Structural problems

### P1 — The pane registry cannot represent a pane rendered twice

`blockComponentModelMap` is keyed by `blockId` alone. The magnify feature
renders the magnified pane **twice at once** (see P2), so two live `Block`
components contend for **one** map slot. When the second copy unmounts on
restore it deletes the shared slot — and the surviving copy never
re-registers. Result: zoom (and anything else resolving via the registry)
silently dies for that pane. Full mechanism in the companion bug spec §2.

This is architectural, not incidental: *any* future feature that renders a
pane in a second surface (picture-in-picture, drag preview, tear-off
ghost) hits the same wall.

### P2 — Magnify is a second render, not a relocation

`MagnifiedPaneOverlay` (`TileLayout.win32.tsx:260`) calls
`layoutModel.renderContent(nodeModel)` to draw the magnified pane a second
time, into an overlay container, while the original tile node stays mounted
under a `tile-hidden` CSS class. So the magnified pane has **two component
trees, two `ViewModel` mount cycles, two registry mounts** for one logical
pane. On restore the overlay copy's cleanup disposes the *shared* view model
and clears the registry slot.

The magnified pane is the *same pane* — it should be the *same component
instance*, merely repositioned. The current design was chosen "to avoid
stacking-context issues that prevent z-index working on tile-nodes" — a real
constraint (see Risks §6), but it traded a CSS problem for a lifecycle bug.

### P3 — Large platform-split duplication

| Module | Files | Total LOC | Actual divergence |
|--------|-------|-----------|-------------------|
| `zoom.{win32,linux,darwin}.ts` | 3 + stub | ~552 | **0 lines of code** — only header comments differ; all three are byte-identical executable code |
| `TileLayout.{win32,linux,darwin}.tsx` | 3 + stub | ~2340 | ~206 lines win32↔linux, ~86 linux↔darwin — real deltas exist (launcher wiring, drag-preview) but `MagnifiedPaneOverlay`, `DisplayNode`, `DisplayNodesWrapper` are copy-pasted 3× |
| `useWindowDrag.{win32,linux,darwin}.ts` | 3 + stub | ~314 | win32/linux diverge; darwin is an 11-line stub |

Consequences:

- **The magnify fix must be pasted into three files.** Three chances to
  drift; the files have *already* drifted (comment rot, 206-line gap).
- `zoom.platform.ts` indirection exists for **zero** code divergence — the
  platform behaviour is entirely in `window-header.{scss,darwin.scss}`.
- A reviewer cannot tell, without diffing, which lines are intentionally
  platform-specific and which are accidental drift.

### P4 — Zoom is fragmented

One behaviour, but: two domains (pane vs chrome) interleaved in one file;
two dispatch mechanisms (the `keymodel` global key map vs an ad-hoc
`window` `wheel` listener mounted by `AppZoomHandler` in `app.tsx`); two
pane-resolution strategies (`getFocusedBlockId()` for keyboard,
`target.closest("[data-blockid]")` for wheel). There is no single "zoom
service" — behaviour and policy are scattered across `zoom.*.ts`,
`keymodel.ts`, and `app.tsx`.

### P6 — Browser panes: the duplicate render is *destructive*, not cosmetic

For a terminal/agent pane the duplicate render (P2) "only" corrupts the
registry → zoom dies, but the pane keeps working. For a **browser pane** the
same duplicate render destroys the pane:

1. Magnify → the overlay mounts a *second* `BrowserViewComponent` for the
   same `block_id`. Its `onMount` fires `browser_pane_create` again — the
   reducer returns `AlreadyLive` (no second HWND, good) — but now **two
   placeholders drive geometry**: `tile-hidden` is `visibility: hidden`, so
   the still-mounted tile placeholder keeps its layout box and reports the
   tile rect, while the overlay placeholder reports the magnified rect. Their
   two 200 ms intervals fire `browser_pane_resize` alternately → the native
   HWND is yanked between the tile rect and the magnified rect every tick.
2. Restore → the overlay `BrowserViewComponent` unmounts → its `onCleanup`
   fires **`browser_pane_close`** → the host reducer flips the *shared*
   `block_id` entry `Live → Closing` and **destroys the native HWND**.
3. The tile `BrowserViewComponent` is still mounted and still believes
   `paneCreated === true`. `createPane` only ever runs once (in `onMount`),
   so it **never recreates** the pane. Its `syncPosition` keeps firing
   `browser_pane_resize` against a `Closing`/destroyed pane — all no-op'd by
   the reducer.

Net result: **the browser pane goes black (HWND destroyed, no compositor)
and is stuck (no recreate path)** — exactly the reported symptom. This is the
direct consequence of (a) the duplicate render and (b) browser-pane lifecycle
being coupled 1:1 to component mount/unmount (§2.4). It is the strongest
argument that the magnified pane *must* be a single component instance.

### P5 — Window maximize emits no state-change event

`maximize_window` toggles via raw `ShowWindow` and returns; the frontend is
never told the new window state, and on `SW_RESTORE` the CEF webview can
lose keyboard focus. Lower priority (the reported bug was pane magnify), but
worth a follow-up: a `window-state-change` event would let the frontend
re-assert focus and update chrome.

---

## 4. Target architecture

### A. One render per pane (fixes P1 + P2 + P6) — mandatory

A pane is rendered by exactly one `Block` component instance. "Magnified" is
a *position/size state* of that instance, not a second instance. Implement
the overlay as a **positioned mount target** and relocate the existing
`DisplayNode` content into it (SolidJS `<Portal>` / DOM reparent), or move
the tile node itself. `tile-hidden` twin and `renderContent`-again go away.
The `ViewModel` lifecycle, the registry slot, and the browser-pane
create/close lifecycle are then all untouched by magnify.

**This is no longer optional.** The reference-count fallback (Option B in the
bug spec) is sufficient for terminal panes but **not** for browser panes:
even with the registry slot ref-counted, the two `.browser-placeholder`
nodes still fight over `browser_pane_resize` while magnified (P6 step 1), and
mount-coupling still means a stray unmount can close the pane. Only a single
render instance — hence a single placeholder, a single create, a single
close — makes browser panes robust. Single-instance is the target for *all*
pane types.

### A′. Browser panes through the reducer (fixes P6)

Single-instance render removes the duplicate `create`/`close`/`resize`, but
two further guarantees should be made explicit so the native pane can never
again be orphaned:

- **Geometry has one authority.** Exactly one `.browser-placeholder` drives
  `browser_pane_resize`. While magnified, that one placeholder reports the
  magnified rect; on restore it reports the tile rect. No hidden/zero-area
  placeholder ever emits geometry.
- **Lifecycle decoupled from transient unmounts.** A browser pane's
  `browser_pane_close` should fire on *logical* pane destruction (the block
  is closed/removed), not on any `BrowserViewComponent` unmount. If a future
  change ever does reparent across a real unmount, route close through a
  reducer command that distinguishes "pane removed" from "pane relocated"
  (e.g. the host reducer already has `Live → Closing`; add a relocate path
  that keeps the entry `Live`).
- **Recovery path.** As defensive depth, `BrowserViewComponent` should
  recreate the native pane if it finds itself mounted with
  `paneCreated === true` but the host reports the `block_id` is `Closed` —
  so a pane can never be permanently black/stuck even if a lifecycle race
  slips through. The host reducer's three-way `TryRegisterBrowserPaneLive`
  (`Fresh`/`AlreadyLive`/`Closing`) already gives the frontend enough signal
  to drive this.

### B. Collapse the zoom platform split (fixes P3, zoom half)

Delete `zoom.linux.ts`, `zoom.darwin.ts`, `zoom.platform.ts`; keep a single
`zoom.ts`. Platform differences already live in
`window-header.{scss,darwin.scss}` and stay there. Net: ~−370 LOC, one file
to maintain. (Confirm `platformResolve` tolerates a non-suffixed module —
Risks §6.)

### C. Extract a shared TileLayout core (fixes P3, layout half)

Move the platform-identical pieces — `MagnifiedPaneOverlay`, `DisplayNode`'s
magnify logic, `DisplayNodesWrapper` — into a `TileLayout.shared.tsx`. The
`TileLayout.{os}.tsx` files keep only genuine platform deltas (launcher
wiring, Win32 drag-preview generation, the dblclick-maximize target). The
magnify fix from §A then lives in **one** place.

### D. A single zoom module (fixes P4)

Consolidate into one `zoom.ts` exposing a small surface:
`paneZoom.{in,out,reset}`, `chromeZoom.{in,out,reset}`, and one
pane-resolution helper. Route the wheel path and the keyboard path through
that same module so policy (clamps, steps, indicator) lives in one place.
Full rewrite is optional; the firm requirement is: **the bug fix must not be
duplicated**, and pane vs chrome zoom must stop being two half-documented
halves of one file.

### E. Disambiguate "maximize" vs "magnify"

Pick one term per mechanism and apply it across UI strings, icon names, and
code identifiers. Recommendation: keep **"magnify"** for the pane action
(matches `magnifiedNodeId`, `layoutMagnify`) and reserve **"maximize"** for
the OS window. Update the pane-header tooltip/icon accordingly.

---

## 5. Recommended sequencing

1. **Zoom platform collapse (B)** — mechanical, low-risk, shrinks the
   surface before any behavioural change.
2. **TileLayout shared-core extraction (C)** — so the magnify fix lands once.
3. **Magnify-as-relocation (A + A′)** — the real fix for the zoom regression
   *and* the browser-pane black/stuck failure. Land the companion spec's
   defaults (full-window size, 100% opacity) in the same pass since they
   touch the same code. Validate explicitly with a browser pane, a terminal
   pane, and an agent pane, magnified and restored repeatedly.
4. **Zoom module consolidation (D)** — opportunistic, after the structure
   is deduplicated.
5. **Window-state event (P5)** — separate follow-up.

> If the team needs the regression fixed *before* C lands, apply the bug
> spec's fix — but accept it will be a 3-file paste and schedule C to
> reclaim it.

---

## 6. Risks & open questions

- **Native child windows (chief risk).** Browser panes are `CefBrowserView`
  native child HWNDs, not DOM. The current separate-overlay render exists
  specifically to dodge "stacking-context issues". A single-instance
  Portal/DOM-reparent must be verified to:
  - keep the *same* `.browser-placeholder` node alive across the magnify
    transition (a reparent that unmounts/remounts the subtree would still
    fire `browser_pane_close`/`create` — the very thing §A′ forbids); and
  - keep the native pane correctly positioned and z-ordered above the
    overlay when magnified.
  If a clean reparent that preserves the subtree proves impossible, the
  fallback is **not** Option B — it is to make magnify a reducer-tracked
  *relocate* of the browser pane (host keeps the entry `Live`, just resizes
  the HWND to the magnified rect) while the DOM still single-renders. The
  reducer must be the source of truth; DOM-driven geometry races must end.
- **`platformResolve` plugin.** Confirm it resolves a plain `zoom.ts`
  (no `.win32`/`.platform` suffix) without complaint before deleting the
  split.
- **TileLayout real deltas.** Audit exactly which of the ~206 win32↔linux
  differing lines are intentional (launcher, drag-preview) vs drift, so the
  shared core does not accidentally erase platform behaviour.
- **Terminology change (E)** is user-visible (tooltip text) — confirm before
  shipping.

---

## 7. Out of scope

- Rewriting the OS window-maximize path beyond adding a state event.
- Changing the magnify blur defaults (`window:magnifiedblockblur*px`).
- The `useWindowDrag` platform split — flagged under P3 but not addressed
  here; fold into C if convenient.
