# Spec: Ctrl+Shift+Scroll zooms every pane in the window at once

**Date:** 2026-09-07
**Status:** Proposed
**Motivated by:** direct request — *"we want to add a feature where
ctrl+shift+scroll anywhere not part of the chrome will cause all the panes
to zoom in/out .. relative to where they are already at ... if over the
chrome, it just acts normal zoom."*

## Problem

Ctrl+Scroll already zooms exactly one target — whichever pane is under the
cursor, or the chrome (title bar + status bar) as a single unit if the
cursor is over `.window-header` / `.status-bar` / `.block-frame-default-header`
(`AppZoomHandler`, `frontend/app/app.tsx:189-227`). There is no gesture that
zooms every pane in the window together. Doing that today means either
repeatedly zooming each pane one at a time, or living with mismatched zoom
levels across panes.

Ctrl+**Shift**+Scroll is completely unused in this codebase — no wheel
handler anywhere checks `shiftKey` (frontend or the native CEF side) — so
this is a free gesture with nothing to preserve or conflict with.

## Design

Add a second wheel handler, `Ctrl+Shift+Scroll`, alongside the existing
`AppZoomHandler`:

- **Over chrome** (same hit-test `AppZoomHandler` already uses:
  `.window-header`, `.status-bar`, `.block-frame-default-header`): behaves
  exactly like plain Ctrl+Scroll over chrome does today — `chromeZoomIn`/
  `chromeZoomOut` (`frontend/app/store/zoom.ts:151-157`). Shift changes
  nothing here; this is "acts normal zoom" from the request.
- **Everywhere else in the window**: step the zoom of **every pane in the
  current window**, each relative to its own current level — not to a
  shared baseline. Implemented as `zoomAllPanesIn`/`zoomAllPanesOut` in
  `zoom.ts`, reusing the existing `stepZoom`/`getBlockZoom` machinery
  rather than a parallel implementation:

  ```ts
  function stepAllPanes(step: number, direction: 1 | -1): void {
      let minZoom = Infinity, maxZoom = -Infinity;
      for (const [blockId] of getAllBlockComponentModelEntries()) {
          const zoom = getBlockZoom(blockId);
          if (zoom == null) continue; // not a zoomable pane type
          stepZoom(blockId, zoom, step, direction, /* showIndicator */ false);
          const newZoom = getBlockZoom(blockId) ?? DEFAULT_ZOOM;
          minZoom = Math.min(minZoom, newZoom);
          maxZoom = Math.max(maxZoom, newZoom);
      }
      if (minZoom === Infinity) return;
      showZoomIndicator(/* one summary toast, see below */);
  }
  ```

  `getAllBlockComponentModelEntries()` is a small new export from
  `block-component-registry.ts` alongside the existing
  `getAllBlockComponentModels()` — needed because `ViewModel`'s base
  interface does not declare `blockId` (each concrete view model carries it
  as its own implementation detail, not a common interface field), so a
  caller that needs both the id and the model per pane cannot recover the id
  from a plain `getAllBlockComponentModels()` value. The registry's map is
  already keyed on blockId; the new export exposes that key instead of
  discarding it, same registry `termViewModel.ts` and `keymodel.ts` already
  walk (by values only) for their own cross-block operations.

  `getBlockZoom`'s existing viewType allowlist (`term`, `agent`, `swarm`,
  `editor`, `armory`) does all the filtering — a browser or warden pane is
  silently skipped, exactly as it would be if you Ctrl+Scrolled over it
  directly today. **Zero new filtering logic.**

  Each pane keeps its own `term:zoom` meta and steps from *its own* current
  value — two panes at different zoom levels before the gesture stay at
  different-but-both-shifted levels after it, which is what "relative to
  where they are already at" means: this is a batch of independent
  single-pane zooms, not a new shared zoom value.

- **Zoom indicator**: `stepZoom`/`setBlockZoom` gained an optional
  `showIndicator` parameter (default `true`, so every existing caller is
  unaffected). The all-panes stepper passes `false` for every individual
  pane and shows exactly one summary toast after the loop —
  `"All panes: 120%"` if every affected pane landed on the same value,
  `"All panes: 90%–130%"` if they didn't — instead of N toasts each
  overwriting the last before it can be read.

### Why this doesn't need a new keydown/preventDefault path

The implementation is a second `wheel` listener, `AppAllPanesZoomHandler`,
with the same shape as `AppZoomHandler` — gated on `(e.ctrlKey ||
e.metaKey) && e.shiftKey` instead of bare `e.ctrlKey`/`e.metaKey`,
registered the same way (`window.addEventListener("wheel", ..., { passive:
false })`) so it can call `preventDefault()`.

### `AppZoomHandler` itself needed a fix too — found during implementation

The spec as originally drafted only flagged the six per-view handlers below
as needing a `shiftKey` exclusion. It missed that `AppZoomHandler`'s own
gate — `if (!e.ctrlKey && !e.metaKey) return;` — never checked `shiftKey`
either. Both handlers are separate listeners on the same `window` "wheel"
event; without excluding Shift from `AppZoomHandler`'s own gate too, **both
handlers fire on Ctrl+Shift+Scroll**, double-stepping whichever single pane
happens to be under the cursor (once from `AppZoomHandler`'s per-pane path,
once again as part of `AppAllPanesZoomHandler`'s loop) relative to every
other pane in the window, which only gets the one step. Fixed by adding the
same `|| e.shiftKey` exclusion to `AppZoomHandler`'s existing gate.

### Interaction with the six duplicated per-view Ctrl+Wheel handlers

`term.tsx:371-386`, `editor-view.tsx:118-131`, `armory-view.tsx:66-80`,
`warden-view.tsx:43-57`, `swarm-view.tsx:30-71`, and
`AgentShellSubblock.tsx:219-234` each register their own capture-phase
Ctrl+Wheel listener that duplicates `zoom.ts`'s stepping logic inline
(documented in full in
[`REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md`](../reports/REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md)
§1). None of them check `shiftKey`, so as written today they would **also**
fire on Ctrl+Shift+Scroll — double-stepping the hovered pane (once from the
per-view handler, once from this feature's window-level loop) while every
*other* pane in the window only gets the one step from the loop.

This needs one of two fixes, and this spec takes the first:

1. **Add a `!e.shiftKey` guard to each of the six handlers.** Small, and
   consistent with how `AppZoomHandler` itself is scoped to bare
   `e.ctrlKey`, not `e.ctrlKey || e.shiftKey`-anything. This is the fix this
   spec implements — six one-line changes, no behavior change to any
   existing shortcut.
2. Consolidate the six onto `zoom.ts`'s shared helpers, per the audit
   report's existing recommendation. Correct long-term direction, but a
   larger, separately-scoped refactor — not a prerequisite for this feature
   and not done as a side effect of it here.

Whichever view is focused when the gesture fires, only option 1 is in scope
for this change.

## Scope decisions

Two forks were resolved with the repo owner before writing this spec, both
towards the smaller/pure-frontend option:

- **Window scope: current window only, not every open AgentMux window.**
  Each AgentMux window is its own renderer process with its own copy of
  `blockComponentModelMap`
  (`docs/specs/browser-pane-state-catalog.md:111,183`) — there is no
  existing mechanism that broadcasts one gesture to every window's panes.
  `getAllBlockComponentModels()` naturally reaches every pane in *this*
  window only, which is exactly this feature's scope: Ctrl+Shift+Scroll in
  window A zooms every pane in window A; a torn-off tab in window B is
  unaffected until you repeat the gesture there.
- **Excludes a browser pane's live rendered page content.** A browser
  pane's actual web page is a separate native view layered on top of the
  app's DOM, not a DOM node the app's JS can see or intercept
  (`SPEC_NATIVE_BROWSER_PANE_2026_04_17.md:32-40`) — confirmed true on every
  platform, not a Windows-only gap. Catching Ctrl+Shift+Scroll over live
  page pixels would require new native (Rust) wheel interception on macOS
  and Linux, where **none exists today** for any modifier
  (`agentmux-cef/src/browser_pane/hwnd.rs:396-402` — this hook only exists
  on Windows, and even there has no Shift check), plus extending the
  Windows hook itself. That is real platform-specific work, tracked as a
  candidate follow-up, not part of this change. A browser pane's own
  chrome — its tab/URL bar — is unaffected either way: that's part of the
  existing single title-bar/status-bar chrome-zoom unit, not per-pane zoom,
  and already zooms today under plain chrome zoom.

## Files

| File | Change |
|---|---|
| `frontend/app/app.tsx` | new `AppAllPanesZoomHandler` (or fold into `AppZoomHandler` as a second branch — implementation's call) registering the Ctrl+Shift+Scroll `wheel` listener |
| `frontend/app/view/term/term.tsx`, `editor-view.tsx`, `armory-view.tsx`, `warden-view.tsx`, `swarm-view.tsx`, `frontend/app/view/agent/components/AgentShellSubblock.tsx` | add `!e.shiftKey` to each handler's existing Ctrl+Wheel gate |
| `frontend/app/store/zoom.ts` | no functional change — reuses `zoomBlockIn`/`zoomBlockOut`/`chromeZoomIn`/`chromeZoomOut` as-is; may add a small suppress/batch helper for the single end-of-gesture indicator |

## Tests

- Ctrl+Shift+Scroll over a non-chrome pane steps **every** pane currently
  registered in the window (term, agent, swarm, editor, armory), each from
  its own starting zoom, by one `WHEEL_STEP`.
- A pane at a different starting zoom than its neighbor ends one step away
  from *its own* prior value, not snapped to a shared value — proves
  "relative to where they already are," not a new global zoom.
- A browser or warden pane in the mix is left untouched by the loop (no
  `SetMetaCommand` call for its block) — proves the existing
  `getBlockZoom` viewType allowlist is doing the exclusion, not new spec-
  specific filtering.
- Ctrl+Shift+Scroll over `.window-header` / `.status-bar` /
  `.block-frame-default-header` calls `chromeZoomIn`/`chromeZoomOut`
  exactly like plain Ctrl+Scroll over chrome does, and does **not** touch
  any pane's `term:zoom`.
- Plain Ctrl+Scroll (no Shift) behavior is unchanged by this feature,
  including inside the six views whose handlers gained a `!e.shiftKey`
  guard.
- Ctrl+Shift+Scroll inside one of those six views' focused pane does not
  double-step that pane relative to its neighbors (regression test for the
  interaction issue above).
- Only one zoom indicator toast is visible per gesture, regardless of pane
  count.

## Non-goals

- **Not cross-window.** A second open AgentMux window (or a torn-off pane
  in one) is untouched by a gesture in the first; see Scope decisions above
  for why, and what broadcasting to every window would require
  (`emit_event_to_window`-style host relay,
  `agentmux-cef/src/events.rs` per
  [`REPORT_BROWSER_PANE_EVENTS_MISROUTED_TO_MAIN_WINDOW_2026_09_06.md`](../reports/REPORT_BROWSER_PANE_EVENTS_MISROUTED_TO_MAIN_WINDOW_2026_09_06.md)).
- **Not a browser pane's live page content.** Needs new native wheel
  interception on macOS/Linux (and a Shift-check addition on Windows) —
  candidate follow-up, not part of this change. Also inherits the existing,
  separately tracked issue #2908 (Ctrl+Wheel zoom doesn't work in floating
  panes) for browser panes specifically — unaffected by this spec either
  way, since browser content is out of scope here regardless.
- **Not a consolidation of the six duplicated per-view zoom handlers.**
  This spec adds the minimum guard (`!e.shiftKey`) each needs to coexist
  with the new gesture; collapsing them onto `zoom.ts`'s shared helpers is
  `REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md`'s recommendation and a
  separate refactor.
- **Not a new persisted "all-panes zoom level."** There is no new meta key
  or setting; this gesture only steps each pane's existing `term:zoom` the
  same way single-pane Ctrl+Scroll already does, just for every pane in one
  motion.
