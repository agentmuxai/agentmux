# Analysis: How should the floating pane header drag the window?

**Date:** 2026-05-27
**Status:** Decision pending; recommendation = Option B

## The problem

After stripping the custom title bar from the floating-pane workspace
(PR #1089), the floater renders the docked-pane's standard
`BlockFrame_Header` as its sole chrome. Dragging the pane header
should **move the floating window** — same as how dragging the main
window's title bar moves it.

What happens instead: pragmatic-dnd's `draggable()` setup on the pane
header fires its HTML5 `dragstart`, the pane drag-out flow kicks in,
and the user gets a **new floating window torn off the original
floating window** — a "double tear-off". The original floater stays
put. Pane content in the second floater is hidden because no body —
the workspace gets two confusing single-pane windows.

Pinned in the latest test session: user reports "if I try to drag the
floating window, it doesnt drag, but creates a new floating window,
like another tear".

## Why HTCAPTION (current attempt) isn't catching the click

PR #1082 added a custom WndProc on the floating-pane class
(`agentmux-cef/src/floating_pane.rs::floating_pane_wndproc`) that
returns `HTCAPTION` for `Y < 33 CSS px` and `(rect.right - x) > 130
CSS px`. The intent: the OS treats that band as the caption, dispatches
the click as a non-client event, owns the drag loop, never lets the
click reach CEF.

The 33 CSS-px figure matches `--header-height` in
`frontend/app/theme.scss:97` — the height of `BlockFrame_Header`
itself. But the pane header isn't at `Y=0` of the window. Between the
window's client-area top and the header's first pixel, there's:

- the TileLayout's outer container padding
- per-tile padding / gap
- the block frame's own outer container (the `.block-frame-default`
  element wraps `.block-frame-default-inner` which contains the
  header)

In practice the header probably starts around `Y=8..16 CSS px` and
ends around `Y=41..49`. My zone covers `Y=0..33` — so the **bottom
portion of the actual pane header** (where most of the title text and
the action buttons sit) is **outside the HTCAPTION zone**. Clicks
there pass through `DefWindowProcW → HTCLIENT → CEF → renderer JS →
pragmatic-dnd → dragstart → tear-off saga.

Bumping the zone to `Y=0..60` would mostly fix it for the current
layout, but the zone is then sensitive to every CSS change in the
tile layout, the block frame, or theme.scss. Brittle.

## The three architectural options

### Option A — OS-level via `WM_NCHITTEST → HTCAPTION`

What we have today (broken). The host's WndProc tells the OS "this
rectangle is the caption"; the OS owns the drag loop.

| | |
|---|---|
| **Pros** | Native, zero IPC overhead — perfectly smooth 60-120 Hz tracking. Free Win32 niceties: ESC cancels drag, Win+arrow snap, drag-to-maximize edge gesture, multi-monitor handoff, cursor capture. Survives JS hangs / slowness. Simpler code (no state machine, no throttling). |
| **Cons** | Hit-test is a coarse Y/X rectangle — can't selectively skip individual buttons inside the zone. **Fragile to layout**: any CSS padding change breaks the zone (current symptom). Mouse events in the zone never reach JS at all — buttons inside HTCAPTION die unless excluded coarsely. Different model from how the main window does drag. |
| **Layout coupling** | High — zone depends on CSS-level Y offsets that no test enforces. |
| **Tear-off conflict** | Resolved cleanly when HTCAPTION fires: pragmatic-dnd's mousedown never runs because the OS intercepts before CEF. Broken when the click misses the zone (today's bug). |
| **Code touchpoints** | `agentmux-cef/src/floating_pane.rs::floating_pane_wndproc` only — ~30 LOC. |

### Option B — JS-driven drag (mousedown + `get/set_window_position` IPC)

Frontend JS listens for mousedown on the header element, queries
current window position, drives moves via IPC. Same pattern as
`frontend/app/hook/useWindowDrag.win32.ts` (the main window's drag).

| | |
|---|---|
| **Pros** | Selective: `target.matches('button, a, input, ...')` cleanly skips interactive elements. Anchors to the **actual header DOM element** via `closest('[data-role="block-header"]')` — no Y-coordinate guesswork, immune to layout padding. **Same pattern as the main window** — consistent codebase. `e.preventDefault()` on mousedown suppresses HTML5 dragstart → blocks pragmatic-dnd's tear-off cleanly without any frontend gating. All header buttons (close, magnify, mic, view-specific endIconButtons) just work. |
| **Cons** | IPC roundtrip on every mousemove (~5-15 ms latency per move). Small but theoretically visible lag — well-mitigated by the existing one-in-flight + coalesce throttling in `useWindowDrag.win32.ts:107-125`. Doesn't get free OS niceties (Win+arrow snap, edge drag-to-maximize). Depends on `find_own_top_level_window` returning the floating window's HWND — works by Z-order (the floater is foreground when you click it), but fragile if a second floater later sits on top. More code (mousedown/move/up state machine — though `useWindowDrag` is a working template). |
| **Layout coupling** | Zero — anchors to a stable DOM attribute (`data-role="block-header"` set in `frontend/app/block/blockframe.tsx:420`). |
| **Tear-off conflict** | Resolved by `e.preventDefault()` on mousedown in the JS handler — HTML5 dragstart never fires, pragmatic-dnd's draggable() never engages. |
| **Code touchpoints** | `frontend/app/workspace/floating-pane-workspace.tsx` (new drag listener, ~60 LOC). `agentmux-cef/src/floating_pane.rs::floating_pane_wndproc` — strip the HTCAPTION block, keep edge resize (~20 LOC removed). |
| **Latent bug** | `set_window_position` / `get_window_position` ignore their `label` arg on Windows (see `agentmux-cef/src/commands/window.rs:223,162`) and use `find_own_top_level_window` — works for the floater because of Z-order but should be fixed properly. Tracked as a follow-up. |

### Option C — Hybrid: thin HTCAPTION moustache + JS for the rest

Add a 4-6 px transparent drag strip above the pane header (or repurpose
unused space) as HTCAPTION; JS handles everything else.

| | |
|---|---|
| **Pros** | Snappy OS-level drag on the thin strip (Win+arrow snap works there). Buttons / clicks elsewhere via JS / pragmatic-dnd. |
| **Cons** | Requires adding a 4-6 CSS px strip that **wasn't in the docked-pane look** — violates the "exactly like a docked pane" requirement. Two drag mechanisms to maintain. Strip placement awkward: above the header looks weird; below it is worse. |
| **Layout coupling** | Medium — strip Y is fixed but its physical px size needs DPI scaling. |
| **Tear-off conflict** | Same as B for the JS-handled regions. |
| **Code touchpoints** | All of A's + all of B's + the strip rendering. |

## Performance reality check

The main window has used Option B (`useWindowDrag`) since the CEF
migration with no user complaints about drag lag. Spec context:
`docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md` documents the
DPI/throttling fixes that landed for this hook. The same hook
(throttled, one-in-flight, DPR-aware) works smoothly on a 4K @ 150%
DPI dev box.

For the floating pane, the move budget is even more forgiving — the
floater is a single block, no other visual chrome to keep in sync.
IPC overhead is not a real concern at this scale.

## Cross-cutting concerns

### The `find_own_top_level_window` problem

Both `get_window_position` and `set_window_position` on Windows
delegate to `find_own_top_level_window`, which does a process-wide
`EnumWindows` and returns the first visible top-level of the current
process. This is the right answer when the user is dragging the
foreground window (which they always are, because they just clicked on
it to start a drag), but it's wrong for any non-Z-topmost window —
e.g. a programmatic move of a non-active floater.

For Option B's normal usage path (user clicks → window comes to top →
user drags), this is fine. But the IPC contract is technically wrong:
both commands accept a `label` argument that's silently dropped on
Windows. Fixing them to honor the label (via
`state.get_browser(label).host().window_handle()`) is a
~10-LOC follow-up that also benefits the main window when multiple
top-level windows exist in the same process.

Tracked but **not blocking** this work.

### Click-through inside HTCAPTION zones

Win32 doesn't let DOM elements rendered by CEF receive mouse events
in an `HTCAPTION` zone — the OS intercepts the click and dispatches
it as `WM_NCLBUTTONDOWN` to the outer WndProc. CEF's child HWND never
sees it. This is what makes Option A fundamentally incompatible with
buttons-inside-the-drag-region.

The only workarounds: native Win32 child controls (not feasible for
CEF-rendered UI), or excluding buttons from the HTCAPTION zone via X
rectangle (what we're doing today for the right-anchored action
cluster, but only works because they're all on one side).

### `data-drag-region` ergonomics for Option B

The main window's `useWindowDrag` walks the DOM looking for
`data-drag-region` ancestors. A floating-pane drag handler should
**not** install a global document-level mousedown listener (we'd drag
the window any time the user clicked any non-button element) — it
should be scoped to the pane header via either:

- A targeted `addEventListener` on the `[data-role="block-header"]`
  element (the approach proposed below).
- OR an explicit `data-drag-region={true}` attribute on the header
  *only when in a floating context*, plus a fix to `isInDragRegion`
  to skip interactive elements (`button, a, input, select, textarea,
  [role="button"]`).

Both are workable; the targeted listener is simpler and doesn't
require threading floating-context awareness through to the shared
`BlockFrame_Header` component.

## Recommendation

**Option B**, scoped via the targeted-listener variant.

### Implementation outline

1. **`agentmux-cef/src/floating_pane.rs`** — strip the `HTCAPTION`
   branch from `floating_pane_wndproc`. Keep the edge resize zones
   (`HT{LEFT,RIGHT,TOP,BOTTOM,corners}`) and `WM_NCCALCSIZE → 0` /
   `WM_NCACTIVATE → 1`. ~20 LOC removed.

2. **`frontend/app/workspace/floating-pane-workspace.tsx`** — install
   a mousedown listener at `onMount` that:
   - Locates the live block header via `document.querySelector('[data-role="block-header"]')`.
   - Or attaches at `document` level but gates on `target.closest('[data-role="block-header"]')`.
   - Skips if the immediate click target is interactive (`target.closest('button, a, input, select, textarea, [role="button"]')`).
   - On qualifying mousedown: `e.preventDefault()` (kills the HTML5
     dragstart that pragmatic-dnd would have used), capture
     `screenX`/`screenY`, IPC `get_window_position` to capture the
     initial window-top-left.
   - On mousemove: compute delta in CSS px, scale by `devicePixelRatio`
     to physical, add to baseline, IPC `set_window_position` (with
     the established one-in-flight + coalesce pattern from
     `useWindowDrag.win32.ts:107-125`).
   - On mouseup: clear the drag state.
   - Cleanup on `onCleanup`.

   ~60 LOC. Mirror of `useWindowDrag.win32.ts` minus the
   data-drag-region traversal (we're scoped explicitly to the pane
   header).

3. **No changes to `BlockFrame_Header`** (`frontend/app/block/blockframe.tsx`)
   — the docked-pane and floating-pane paths render the exact same
   component. Window-drag behavior is opted-in by the floating-pane
   workspace via the listener, not by the header itself.

4. **No changes to pragmatic-dnd setup** in `TileLayout.win32.tsx` —
   `e.preventDefault()` on mousedown in the floating-pane listener is
   enough to block the HTML5 dragstart pragmatic-dnd relies on.

### Net effect

- Drag the pane header (title-text area) → window moves smoothly.
- Click close / magnify / mic / endIconButton → button fires normally.
- Click anywhere in the block body → reaches the block normally (no
  drag, no tear-off, no surprises).
- Resize at edges → still works (kept the edge resize zones in the
  WndProc).
- No more accidental "double tear-off".

## What this analysis does NOT fix

- The `find_own_top_level_window` label-vs-Z-order semantic. Tracked
  as a follow-up: make `set_window_position` / `get_window_position`
  honor the `label` argument on Windows via
  `state.get_browser(label).host().window_handle()`.
- Win+arrow snap and drag-to-maximize gestures on the floater. With
  Option B we forgo these. If desired, Option C can be added later
  as a thin HTCAPTION strip on a non-visible part of the chrome (e.g.
  the very top 4 px of the window, below DwmExtendFrameIntoClientArea's
  reach).
- The VS Code drag-accept cursor problem (separate analysis at
  `docs/analysis/ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md`).
- Re-dock (Phase 4 per spec #810): dragging a floater back into the
  source window's tile layout to re-dock. Not yet implemented.

## Cross-references

- `frontend/app/hook/useWindowDrag.win32.ts` — main-window drag
  reference implementation (one-in-flight + coalesce, DPR scaling,
  catch-up move on IPC resolution).
- `frontend/layout/lib/TileLayout.win32.tsx:443-471` — pragmatic-dnd
  `draggable()` setup on the pane header that we want to suppress
  for floating-context drags.
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` — orchestrates
  the cross-window tear-off saga; not changed by this work but its
  `dragend` listener is what would have fired the double-tear-off.
- `agentmux-cef/src/floating_pane.rs::floating_pane_wndproc` —
  current WndProc. Lines to remove: the `HTCAPTION` branch within
  `WM_NCHITTEST`. Lines to keep: `WM_NCCALCSIZE`, `WM_NCACTIVATE`,
  edge resize zones.
- `agentmux-cef/src/client/wndproc.rs` — main window's
  `install_frameless_resize_hook` (similar pattern, useful reference
  for the edge resize zones).
- `agentmux-cef/src/commands/window.rs:162,223,260` —
  `get_window_position`, `set_window_position`, `start_window_drag`
  IPC handlers; the latent `label`-vs-`find_own_top_level_window` bug
  lives here.
- `frontend/app/block/blockframe.tsx:420` — `data-role="block-header"`
  attribute used to anchor the drag listener.
- `frontend/app/theme.scss:97` — `--header-height: 33px`.
- PR #1082 — original Phase 6 polish (HTCAPTION approach).
- PR #1089 — chrome cleanup (drop custom title bar, drop `WS_CAPTION`,
  add `DwmExtendFrameIntoClientArea`, auto-close on empty workspace).
