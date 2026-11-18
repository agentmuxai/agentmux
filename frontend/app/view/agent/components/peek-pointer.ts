// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the pointer last was, for a hover panel whose row never saw it move. A wheel notch scrolls a different row
 * under a still pointer: that row is hovered, but only the row the pointer moved over hears a `mousemove`, so its
 * panel has no pointer Y of its own and would sit level with the pointer (on top of it). The document hears every
 * move and every notch, wherever the pointer is, so one listener for the whole app keeps the answer.
 */

let lastY: number | null = null;
let installed = false;

function remember(e: MouseEvent) {
    lastY = e.clientY;
}

function install() {
    if (installed || typeof document === "undefined") return;
    installed = true;
    // Passive and capturing: it only reads, and sees the event before anything that stops it.
    document.addEventListener("mousemove", remember, { capture: true, passive: true });
    document.addEventListener("wheel", remember, { capture: true, passive: true });
}

/** A row's layout edge is fractional and the browser's hit test rounds it: a pointer it counts as on the row can lie this far outside the rect. */
const EDGE_SLACK_PX = 1;

/** The pointer's last Y if it lies within [top, bottom] (a row's own extent, give or take the edge's rounding), else null. Installs the listener on first use. */
export function pointerYWithin(top: number, bottom: number): number | null {
    install();
    return lastY != null && lastY >= top - EDGE_SLACK_PX && lastY <= bottom + EDGE_SLACK_PX ? lastY : null;
}

/** Start listening now, so a pointer that moved before any panel was asked is still known. */
export function trackPointer() {
    install();
}

/** Tests: forget the position (the listener stays). */
export function resetPointerForTest(y: number | null = null) {
    lastY = y;
}
