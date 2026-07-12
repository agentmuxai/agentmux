// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Zero-dependency module-level flag: is a pane (tile) drag currently in
// flight? Set by TileLayout's DisplayNode at drag start; cleared at the
// draggable's own onDrop (dragend), the Win11 swallowed-dragend safety
// net, and the tab bar's end-of-drag cleanup layers.
//
// Exists because `currentDragPayload` is deliberately cleared at DROP
// time (so CrossWindowDragMonitor's dragend listener skips handled
// drops) — leaving a ~2ms window between drop and dragend where the
// payload says "no drag" but the drag source element must still not be
// unmounted (removing it suppresses dragend entirely and wedges
// pragmatic's teardown chain). pruneDanglingLeaves gates on THIS flag,
// which spans the full gesture.
//
// Lives in its own module (not crossTabDrag/TileLayout) to stay out of
// the layoutPersistence ↔ crossTabDrag ↔ TileLayout import cycle.

let tileDragInFlight = false;

export function setTileDragInFlight(inFlight: boolean): void {
    tileDragInFlight = inFlight;
}

export function isTileDragInFlight(): boolean {
    return tileDragInFlight;
}
