// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CSS-px depth of the invisible edge grab band on a floating pane.
 *
 * A floater is a frameless WS_POPUP; native edge-resize via the parent wndproc
 * never fires (the embedded CEF child consumes WM_NCHITTEST), so the resize is
 * driven from the DOM: `floating-pane-workspace.tsx` detects a pointerdown
 * within this many CSS px of an edge and drives `set_window_rect`.
 *
 * Nothing is painted at this width — it's purely a hit-test zone — so it can be
 * widened freely for an easier grab target.
 *
 * A floating *browser* pane's web-content is a separate OS child window layered
 * over the floater's frontend DOM, so it would cover this band and swallow the
 * pointerdown. `browser-view.tsx` therefore insets that child by this same
 * value on the three window-edge sides (left/right/bottom — the top edge is
 * over the header, already frontend), exposing the band underneath. Sharing one
 * constant keeps the detector and the inset from ever drifting.
 *
 * SPEC_FLOATING_PANE_EDGE_RESIZE.
 */
export const FLOATER_EDGE_RESIZE_BORDER = 12;
