# Retro: Linux pane tear-off triggers from any click-drag, not just the header

**Date:** 2026-06-20
**Status:** Fix planned (see below)

---

## The problem

On Linux, clicking and dragging anywhere on a pane body — terminal text, browser content, agent transcript — triggers a window tear-off. Tear-off must only activate when the drag originates on the pane's title bar / header strip. On macOS and Windows this works correctly.

---

## How it started

### PR #180 — text selection fix (correct)

`dd5268c7 fix: restrict pane drag to header — fix text selection (#180)`

Added `dragHandle` to pragmatic-dnd's `draggable()` call, passing the header element ref. This correctly restricted drag to the header on all platforms, fixing text selection inside panes.

### PR #182 — the regression

`1d3679be fix: restore pane drag on Linux (WebKitGTK dragHandle incompatibility) (#182)`

WebKitGTK does not support HTML5 DnD when `draggable="true"` is set on a child inside a `draggable="false"` parent. pragmatic-dnd's `dragHandle` option does exactly that: it sets `draggable="true"` on the handle and `draggable="false"` on the tile root. On Linux this silently broke pane drag entirely, so the fix reverted to `element: tileNodeRef, dragHandle: undefined` — the entire tile became the drag surface again. The platform-specific file `TileLayout.linux.tsx` was introduced here.

At the time there was no tear-off feature, so the consequence was just "text selection still broken on Linux" — annoying but not destructive.

### PR #188 — tear-off

`ad10cbfc feat: cross-window drag tear-off, correct tear-off content, maximize drag region (#188)`

Tear-off was wired to pragmatic-dnd's `onDragStart` inside the same tile registrations. Because the Linux tile registered on `tileNodeRef` (the full pane), any drag anywhere now fires tear-off.

### PR #1610 — tear-off pool on macOS/Linux

`ef08433c feat(pool): pane tear-off pool on macOS/Linux (#1610)`

Made tear-off a polished user feature on Linux — which made the whole-tile drag regression immediately visible and annoying.

---

## Why the Windows fix didn't land on Linux

PR #182's commit message correctly identified the WebKitGTK constraint and documented it. But it chose the wrong escape hatch: widening the drag target to the whole tile rather than avoiding the `draggable="false"` parent entirely.

Windows discovered the same constraint later (WebView2 has the same `draggable="true"` child / `draggable="false"` parent limitation) and solved it differently in `TileLayout.win32.tsx`:

> Instead of passing `dragHandle` to `draggable()` (which sets draggable="false" on the tile), register `draggable()` directly on the header element. Only the header gets `draggable="true"`; the tile root gets no draggable attribute at all, which defaults to non-draggable. No `draggable="false"` on any parent → no WebView2/WebKitGTK incompatibility.

This pattern was never backported to the Linux file.

---

## Fix plan

**File:** `frontend/layout/lib/TileLayout.linux.tsx`, lines 351–416.

**Change:** Mirror the win32 pattern exactly:

1. Replace `element: tileNodeRef, dragHandle: undefined` with registration on the live header element found via `tileNodeRef.querySelector('[data-role="block-header"]')`.
2. Add the polling loop (`setInterval(register, 100)`) from win32 — the header is not in the DOM at tile mount time (block content loads async behind a SolidJS `<Show>` gate).
3. Guard re-registration against active drags (same guard as win32: if `props.layoutModel.activeDrag()` is true, skip the re-register to avoid blowing away pragmatic-dnd's onDrop listener mid-drag).
4. Keep `dragHandle: undefined` (or omit it) — we're registering on the header element itself, so there is no parent to suppress.
5. Drop the `dragHandleRef` variable (line 360) — it's unused after this change.
6. Update the top-of-file comment to explain the new approach.

No changes needed outside `TileLayout.linux.tsx`. No Rust changes. No new dependencies.

**Why this is safe on WebKitGTK:**
- pragmatic-dnd sets `draggable="true"` on the registered `element` only.
- When `element` is the header, the header gets `draggable="true"`.
- `tileNodeRef` (the parent) gets no `draggable` attribute — defaults to `false` implicitly.
- WebKitGTK's constraint is specifically about an *explicit* `draggable="false"` attribute on a parent. A parent with no attribute is fine.

---

## Lessons

1. **"Can't use dragHandle" ≠ "use full-tile drag."** The correct escape hatch is to register on the target element directly, not to widen the drag surface. Both WebKitGTK and WebView2 have the same constraint; Windows found the right fix later and it should have been backported immediately.

2. **Per-platform files create a backport debt.** The three `TileLayout.{linux,darwin,win32}.tsx` files diverge silently. When win32 fixes something that applies to Linux, there's no automated signal. A shared comment block or a cross-reference in the file headers would reduce drift.

3. **"Annoying but not destructive" bugs compound.** The Linux whole-tile drag was accepted as a known regression after #182 because pane DnD worked at all. The tear-off feature in #188 silently promoted that regression into a destructive UX issue (users losing pane position on every accidental drag). Known regressions should be tracked as issues, not left as comments in source.
