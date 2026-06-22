# Floating-pane DnD lifecycle — architecture rethink

**Date:** 2026-06-22
**Status:** Proposed (design)
**Context:** Issue #1662, PR #1677. Written after a long reactive patching session
established that the floating-pane drag-and-drop lifecycle is structurally
fragile and needs design, not more point-fixes.

---

## 0. Why this exists

A series of symptom-level fixes (redock-leaf insert, redock guard, block-move
guard, dark floater background, deferred floater show) each fixed one visible
bug and several introduced new ones. PR #1677 ships the subset that is confirmed
stable **on a single window**; this doc captures the root causes so the
remaining work is built once, deliberately, with tests.

Confirmed-stable and shipping in #1677: floater independence, tear-off un-jam,
move-doesn't-delete-the-block (via a band-aid guard), darker floater.

Open / frontier (this doc):
1. **2nd-window tear-off with the tear-off pool** — a brand-new path. The pool
   feature originally blanked secondary windows; #1654 fixed the blank render;
   tear-off *from* a pool-promoted secondary window has never been validated and
   currently fails (`TearOffBlock: block not found` — a frontend/backend block
   desync; the block also fails to *persist* torn off).
2. **The remaining floater white flash** on tear-off.

---

## 1. Root causes (each spawns multiple symptoms)

### R1 — `onNodeDelete` conflates "moved" with "closed"
`TileLayout` fires a single `onNodeDelete(blockId)` callback whenever a layout
node is removed, and `tab/tabcontent.tsx` unconditionally calls
`ObjectService.DeleteBlock`. But a node is removed in **two semantically
opposite** situations:
- the user **closed** the pane → the block *should* be deleted;
- the block was **moved** (tear-off, redock, cross-window drop) → the node is
  removed from the source but the block must **survive**.

Every move removes a source node, so it always races `DeleteBlock` against the
move RPC (`TearOffBlock` / `RedockFloatingPane`). Symptoms: redock empty-slot,
logo-only floater on tear-off, block "snapping back", `block not found`.

The shipped **block-move guard** (`block-move-guard.ts`) is a renderer-local,
time-boxed suppression of `DeleteBlock` during a move. It works for the cases
tested but is inherently racy (wall-clock window, renderer-scoped) and is the
prime suspect for the residual flakiness.

**Durable fix:** make node removal carry **intent**. A
`LayoutTreeDeleteNodeAction` (and the cross-window/redock paths) should pass a
`reason: "close" | "move"`. `onNodeDelete` deletes the block only on `"close"`.
Then **delete the guard entirely**. This single change dissolves R1's whole
symptom cluster without timers or renderer-local state.

### R2 — the floater is a raw Win32 popup running the browser-pane handler
The main window is a clean **CEF Views** window: it is shown only at
`on_load_end` (no white flash) and CEF handles frameless/resize/snap. The
floater is a `CreateWindowExW` popup with a CEF child embedded via
`set_as_child`, created with `new_with_browser_pane` (`is_browser_pane = true`).

Consequences:
- **White flash:** the popup is `ShowWindow`n before its child paints; the
  transparent global `CefSettings.background_color` (intentional, for the
  frameless cascade) + the DWM-extended frame glass composite to white for a
  frame. The dark brush + opaque child background remove one frame; the other
  needs a paint-gated reveal. A naive "hide until `on_load_end`" reveal was
  tried and **reverted** — it left floaters hidden/blind (the floater's
  pane-handler makes `on_load_end` take the pane early-return) and broke
  tear-off.
- **Lifecycle quirks:** the pane-handler path, the floater registry
  (`ACTIVE_FLOATER_HWNDS`), and the bespoke `floating_pane_wndproc` all exist to
  emulate what CEF Views gives the main window for free.

**Durable fix:** make floaters **CEF Views windows** like the main window
(frameless Views window hosting the floating workspace), so deferred-show,
transparency, and resize all come for free and the flash disappears at the
source. Larger change; retires `create_popup`, the floater wndproc, and the
HWND registry.

### R3 — pool-promoted secondary windows have no stable identity
Promoted pool windows keep their `window-pool-*` label. We have already hit
several resolution bugs from this (redock resolving to `"main"`,
`find_main_window` returning the wrong window). The 2nd-window-tear-off
`block not found` is most likely the same class: a pool-promoted window's
workspace/block routing doesn't line up with what `TearOffBlock` expects.

**Durable fix:** re-key a pool window to a real, stable window identity at
**promote** time (label + `window_hwnds` + `backend_window_id`), so secondary
windows are indistinguishable from a cold-created window to every downstream
consumer (tear-off, redock-resolve, focus, close).

---

## 2. Suggested sequencing

1. **R1 first** (highest leverage, smallest change): intent on node removal →
   delete the block-move guard. Re-validate redock + tear-off persistence.
2. **R3**: pool-window identity re-key at promote → fixes 2nd-window tear-off
   and the redock/focus resolution bugs.
3. **R2**: CEF-Views floaters → kills the flash and retires the raw-popup
   machinery. Largest; do last, gated on its own spec.

Each phase is independently shippable and testable. R1+R3 together should make
2nd-window-tear-off-with-pool work; R2 is the polish/cleanup tier.

## 3. Test gaps to close
- Tear-off **persists** (block stays in the floater workspace; source layout
  has no orphan node) — automated where possible.
- Redock renders in the target and the block survives — repeat N times.
- 2nd-window (pool-promoted) tear-off + redock — the frontier path.
- No white frame on floater show (perceptual / screenshot-diff, like the
  pool-window blank-render harness in `docs/plans/PLAN_POOL_HIDDEN_PREWARM`).
