# Fix: Tab/Pane Drag Cursor — Circle-Slash → Plus Sign

**Branch:** `agenta/fix-tab-drag-cursor`
**Date:** 2026-03-24

---

## Problem

When dragging a tab (or pane header) outside the AgentMux window toward the desktop, the cursor shows **circle-slash** (🚫). It should show **arrow + plus** (➕) to communicate "drop here opens a new window."

## Root Cause

`dataTransfer.effectAllowed` is never set during `dragstart`. Windows OLE defaults to the forbidden cursor when there is no drop target (i.e., outside WebView2). Setting `effectAllowed = "copy"` constrains the drag to copy-semantics, and OLE renders the plus cursor over non-WebView2 areas.

The Atlaskit PDND `draggable()` adapter does not expose `dataTransfer` directly, so `effectAllowed` must be set via a native `dragstart` listener on the draggable element. Because Atlaskit registers its own handler on the document in **capture phase**, a **bubble-phase** listener on the element fires after Atlaskit has committed the drag — within the same event, so `effectAllowed` is still writable.

## Changes

### `frontend/app/tab/droppable-tab.tsx`
Add a native `dragstart` listener on `tabWrapRef` that sets `effectAllowed = "copy"`.

### `frontend/layout/lib/TileLayout.win32.tsx`
Same fix on the pane header element (`handle`) inside `register()`.

## Scope

- Cursor UX only — no tearoff logic, no Atlaskit internals touched.
- Linux and macOS do not have the OLE cursor issue; no changes needed there.
