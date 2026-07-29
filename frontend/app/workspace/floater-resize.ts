// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CSS-px depth of the invisible edge grab band on a floating pane.
 *
 * A floater is a frameless WS_POPUP; native edge-resize via the parent wndproc
 * never fires (the embedded CEF child consumes WM_NCHITTEST), so the resize is
 * driven from the DOM: `floating-pane-workspace.tsx` detects a pointerdown
 * within this many CSS px of an edge and drives `set_window_rect`. This applies
 * on all three platforms — Windows, macOS, and Linux all run the same
 * DOM-driven detection (see `posScale()` in `floating-pane-workspace.tsx` for
 * the per-platform coordinate-space handling).
 *
 * Nothing is painted at this width — it's purely a hit-test zone — so widening
 * it costs nothing visually *except* for floating browser panes (see below).
 *
 * A floating *browser* pane's web-content is a separate OS child window layered
 * over the floater's frontend DOM, so it would cover this band and swallow the
 * pointerdown. `use-pane-rect-sync.ts` therefore insets that child by this same
 * value on the three window-edge sides (left/right/bottom — the top edge is
 * over the header, already frontend), exposing the band underneath. Sharing one
 * constant keeps the detector and the inset from ever drifting — but it also
 * means this value is a genuine tradeoff between two competing pane types:
 * wider is strictly better for grabbability everywhere, but for browser panes
 * specifically, the inset exposes the floater's own background as a matte
 * around the web content, and a wide matte reads as an unwanted border.
 *
 * History: shipped at 12px (PR #1177, 2026-05-29) — a comfortable grab target
 * with no reported issues for non-browser floaters. PR #1829 (2026-06-29)
 * treated the *browser-pane matte* complaint as a reason to shrink the *shared*
 * constant, first to 6px then to 4px in the same PR — which fixed the matte at
 * the cost of making every floating pane (agent, terminal, editor, etc., none
 * of which have this matte problem at all) hard to grab. Restored to 8px: a
 * deliberate middle point, roughly double the unusable 4px and comfortably
 * under the 12px that read as a border for browser panes. Matches the
 * tiled-layout splitter's hit-target (`layoutModel.ts`'s `resizeHandleSizePx`,
 * effectively 6px at default settings) in the same spirit, sized slightly
 * larger here specifically because floating panes have no visible hover line
 * to help a user find the zone the way the tiled splitter's 2px accent line
 * does — see `docs/retro/retro-floating-pane-resize-hit-target-2026-07-27.md`.
 *
 * SPEC_FLOATING_PANE_EDGE_RESIZE.
 */
export const FLOATER_EDGE_RESIZE_BORDER = 8;
