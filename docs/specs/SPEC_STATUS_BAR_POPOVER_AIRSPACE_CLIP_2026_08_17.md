# SPEC — Status bar popovers: fix native browser-pane airspace occlusion

**Date:** 2026-08-17
**Type:** Bug fix (root-caused, implementation-ready — no code shipped yet)
**Status:** Draft
**Scope:** `frontend/app/statusbar/HostPopover.tsx`, `frontend/app/statusbar/BackendStatus.tsx` (+ matching CSS in `StatusBar.scss`). No backend changes.

## Problem

When a browser pane occupies part of the window, two status-bar dropdowns —
the host-info popover (click the hostname chip) and the backend-status
popover (click the backend status dot / uptime) — render **behind** the
pane's content instead of over it, if the popover's on-screen position
overlaps the pane. Every other status-bar popover (CPU cores, disk volumes,
token-usage breakdown, the version/instance panel) and the widget bar's
"More ▾" dropdown paint correctly above the pane. Only these two are broken.

## Root cause

On Windows, a browser pane's content is a real native child HWND
(`agentmux-cef`'s `CefBrowserView`), which Win32 composites **above the DOM
regardless of CSS z-index** — the "airspace problem," fully diagnosed in
`docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`. Any DOM element that
needs to visually sit above a pane must explicitly register its screen rect
so the backend can punch a transparent hole through the pane's HWND
(`SetWindowRgn`) at that rect. There is no way for CSS alone to win this —
raising `z-index` does nothing, because the HWND isn't part of the DOM's
paint order at all.

The app already has a working, reusable fix for exactly this, used by
**every other** status-bar popover:

- `usePaneOverlay(() => rootRef)` (`frontend/app/platform/pane-overlay.ts:265-316`)
  — call once per popover root; it measures the element's
  `getBoundingClientRect()` on mount/resize/reposition and dispatches the
  rect into the shared overlay-clip map, which flushes to the
  `browser_panes_set_overlay_clip` IPC. This is the primitive that actually
  makes the airspace transparent, and it works cross-platform (on
  non-Windows the same dispatch drives the host's whole-pane-hide fallback,
  `pane-overlay.ts:172-203`).
- `data-pane-overlay` attribute on the popover's root element — belt-and-
  suspenders auto-discovery via `pane-overlay-auto.ts`, which independently
  re-measures on style/resize mutations. **This auto-service only starts on
  Windows** (`pane-overlay-auto.ts:130`), so it is not a substitute for the
  `usePaneOverlay()` hook call above on other platforms — both must be
  present, matching every existing consumer.
- Render via `<Portal>` (mount to `document.body`) instead of nesting the
  popover inside the trigger's own DOM subtree, positioned with
  `computeMenuPosition`/`autoUpdate` (floating-ui) anchored to the trigger's
  rect.

Confirmed consumers of this exact combination:
`CpuCoresPopover.tsx:21,57` + `SystemStats.tsx:202-207` (Portal),
`DiskVolumesPopover.tsx:18,39` + `SystemStats.tsx:269-275` (Portal),
`TokenBreakdownPopover.tsx:16,54,127` + `TokenUsageIndicator.tsx:95-101`
(Portal), `InstancePanel.tsx:23,50` (rendered via fixed positioning,
`StatusBar.tsx:116-118`), and the widget bar's `PopoverMenu`
(`frontend/app/element/popover-menu.tsx:5,52,95-106` — the mechanism behind
"More ▾", which is why it already paints correctly). `CpuCoresPopover.tsx:15-16`'s
own doc comment calls `TokenBreakdownPopover` "the canonical status-bar
popover" for this pattern.

**`HostPopover.tsx` and `BackendStatus.tsx` never adopted this pattern.**
Both render their dropdown as a plain nested `<div class="status-bar-popover">`
(`HostPopover.tsx:167`, `BackendStatus.tsx:169`) inside a
`position: relative` wrapper (`HostPopover.tsx:144`, `BackendStatus.tsx:145`),
positioned purely with CSS (`StatusBar.scss:286-299` —
`position: absolute; bottom: calc(100% + 4px); left: 0;`). Neither calls
`usePaneOverlay()`, neither sets `data-pane-overlay`, neither is rendered
through a `Portal`. Their screen rect is never registered with
`pane-overlay.ts`'s clip map, so the backend never punches a hole for them
and the pane's HWND keeps painting over them uninterrupted — exactly the
airspace bug the other five components above were already fixed for.

`ConfigStatus.tsx`, `UpdateStatus.tsx`, and `GpuStatus.tsx` were checked and
confirmed to render no popover of their own (`ConfigStatus` opens a modal via
the existing `openModal`/`ModalLayer` machinery, which already handles this;
`UpdateStatus`/`GpuStatus` have no dropdown at all) — they are out of scope,
not overlooked.

## Design

Migrate `HostPopover.tsx` and `BackendStatus.tsx` to the same shape as
`TokenUsageIndicator.tsx` + `TokenBreakdownPopover.tsx`, since both are a
single trigger + single popover with an outside-click/Esc dismiss — the
closest existing precedent, closer than the CPU/disk pair (which share a
parent `SystemStats.tsx` for two triggers) or `InstancePanel` (which is
driven by `StatusBar.tsx` itself).

For each of the two components:

1. **Split into trigger + portaled popover**, same shape as
   `TokenUsageIndicator.tsx:74-104`: the trigger keeps its existing
   `status-bar-item` markup; the popover moves into `<Show when={open()}><Portal><... /></Portal></Show>`.
2. **Popover root calls `usePaneOverlay(() => rootRef)`** and sets
   `data-pane-overlay` on its outermost element (mirrors
   `TokenBreakdownPopover.tsx:54,127`).
3. **Position via `computeMenuPosition` + `autoUpdate`**
   (`TokenBreakdownPopover.tsx:74-107`), anchored to the trigger's
   `getBoundingClientRect()` captured on open (already done —
   `HostPopover.tsx` has no `anchorRect` state today and will need one,
   `BackendStatus.tsx` likewise). Use placement `"top-start"` for both
   (they're left-aligned status-bar-left items in `HostPopover`'s case, and
   `BackendStatus` is also left-side) to preserve the current
   `bottom: 100%; left: 0` visual alignment — contrast with
   `TokenBreakdownPopover`'s `"top-end"`, which is right-aligned because
   `TokenUsageIndicator` sits in `status-bar-right`
   (`StatusBar.tsx:66-78`). Pass `avoidNativePanes: false` (the popover is
   *meant* to sit over a pane now, not dodge it — same reasoning as
   `TokenBreakdownPopover.tsx:104-106`'s comment on why
   `assertMenuInPaintableArea` is intentionally omitted there).
4. **Fix outside-click detection for the new dual-ref shape.** Both
   components currently detect outside clicks by checking
   `!popoverRef.contains(e.target)` where `popoverRef` is the *shared*
   wrapper containing both the trigger and the popover
   (`HostPopover.tsx:132-140`, `BackendStatus.tsx:132-141`). Once the
   popover is portaled out of that wrapper, this check no longer includes
   clicks inside the popover itself and would immediately self-dismiss on
   any click inside it. Switch to the dual-ref pattern already used by
   `TokenUsageIndicator.tsx:50-59`: keep separate `triggerRef`/`popoverRef`
   and check `triggerRef?.contains(t) || popoverRef?.contains(t)`.
5. **CSS**: the popover keeps its existing `.status-bar-popover` visual
   styling (background/border/padding/rows) from `StatusBar.scss:286-320` —
   only the *positioning* properties (`position`, `bottom`, `left`,
   `z-index`) stop being needed on that class once floating-ui supplies
   `position: fixed; left/top: ...px` inline via `computeMenuPosition`'s
   returned style, same as `TokenBreakdownPopover`'s `floatingStyle()`
   (`TokenBreakdownPopover.tsx:75-79,128`). No other visual change.

No change to `HostPopover`'s QR-code canvas, LAN-toggle checkbox, or
MuxBus sign-in controls, or to `BackendStatus`'s GPU/backend-death detail
rows — all of that content is unaffected by where in the DOM tree the
popover mounts.

## Scope / non-goals

- No visual/design change — same popover content, same rows, same
  open/close triggers. This is purely a "make it paint in the right place"
  fix.
- Does not touch `pane-overlay.ts`/`pane-overlay-auto.ts` themselves — the
  primitive is correct and already proven by five other consumers; this
  spec only wires two stragglers onto it.
- Does not address the browser-pane loading-indicator flicker — unrelated
  bug, covered separately in
  `docs/specs/SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md`.

## Verification

Manual repro (no automated test currently exercises native-pane airspace —
consistent with how the CPU/disk/token popovers were verified per their own
specs): open any browser pane (e.g. a messenger widget) so it covers the
bottom-left/bottom-right corners of the window where these two chips live,
then click the hostname chip and the backend-status dot in turn and confirm
each popover is fully visible over the pane's content, not clipped behind
it. Also confirm outside-click-to-close and Esc-to-close still work post-
Portal (the dual-ref fix in step 4 above is what this specifically
exercises). Repeat with the window resized/moved mid-open to confirm
`autoUpdate` keeps the popover anchored (same behavior `TokenBreakdownPopover`
already relies on).

## References

- `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md` — origin of the
  `data-pane-overlay` auto-discovery mechanism and the airspace problem
  writeup.
- `frontend/app/platform/pane-overlay.ts`, `pane-overlay-auto.ts` — the
  primitive this spec reuses unmodified.
- `frontend/app/statusbar/TokenUsageIndicator.tsx`,
  `TokenBreakdownPopover.tsx` — the pattern being mirrored.
- `frontend/app/statusbar/CpuCoresPopover.tsx`,
  `DiskVolumesPopover.tsx`, `SystemStats.tsx` — a second working precedent
  (two triggers sharing one parent).
- `frontend/app/element/popover-menu.tsx` — the widget bar's "More ▾"
  dropdown, the user's own reference point for "already works correctly."
