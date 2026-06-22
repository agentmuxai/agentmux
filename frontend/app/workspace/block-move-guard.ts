// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renderer-local guard that suppresses the generic
 * `onNodeDelete -> ObjectService.DeleteBlock` teardown (wired in
 * `tab/tabcontent.tsx`) WHILE a block is being MOVED between tabs/windows —
 * i.e. a tear-off (pane -> floater) or a redock (floater -> window).
 *
 * Why: both moves first relocate the block to another tab (`TearOffBlock` /
 * `RedockFloatingPane`) and then REMOVE the source window's layout node for it.
 * Removing the node fires the TileLayout's `onNodeDelete` hook, which calls
 * `DeleteBlock` — DESTROYING the very block that was just moved. The result is
 * an empty slot / logo-only floater: a leaf pointing at a deleted block (host
 * log: `WaveObj deleted block:…` racing `workspace.TearOffBlock` /
 * `RedockFloatingPane`). It's a RACE — the node removal can fire before the
 * frontend even sees the block's new parent — so it can't be distinguished
 * after the fact by the block's parent ref. The initiating window must mark
 * "a move is happening" up front, and `onNodeDelete` must skip the delete while
 * it's set. See issue #1662.
 *
 * Scope is the renderer that initiates the move (the source window for a
 * tear-off; the floater for a redock), which is exactly the renderer whose
 * `onNodeDelete` fires for the removed source node. A time-boxed window
 * (auto-expiring) guarantees the guard can never get stuck and permanently
 * swallow a genuine pane close.
 */

let guardUntilMs = 0;

/**
 * Open the guard for `graceMs`. Call at the very start of a move (before any
 * node removal / `onNodeDelete` can fire). The grace window must span the move
 * RPC plus the source-node removal that follows it.
 *
 * Redock closes the floater afterward (its renderer dies), so a longer grace is
 * harmless; tear-off keeps the source window open, so its grace should be just
 * long enough to cover the immediate node-removal microtask.
 */
export function beginBlockMoveGuard(graceMs = 3000): void {
    const until = performance.now() + graceMs;
    if (until > guardUntilMs) guardUntilMs = until;
}

/** Close the guard immediately — call when a move attempt resolved to NO move
 * (e.g. a redock dropped on the desktop) so a later genuine pane close in this
 * still-open window isn't wrongly suppressed. */
export function endBlockMoveGuard(): void {
    guardUntilMs = 0;
}

/** True while a block move is in flight in this renderer. */
export function isBlockMoveGuardActive(): boolean {
    return performance.now() < guardUntilMs;
}
