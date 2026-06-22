// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renderer-local guard that suppresses the generic
 * `onNodeDelete -> ObjectService.DeleteBlock` teardown (wired in
 * `tab/tabcontent.tsx`) WHILE a floating-pane redock is in flight.
 *
 * Why: redocking a floater moves its block into another window's tab
 * (`RedockFloatingPane`) and the floater is dismissed. As part of that, the
 * floater's layout node for the block is removed, which fires the TileLayout's
 * `onNodeDelete` hook — and that hook calls `DeleteBlock`, DESTROYING the very
 * block that was just redocked (host log: `WaveObj deleted block:…` racing
 * `workspace.RedockFloatingPane`). The result is an empty slot in the target
 * window: a leaf pointing at a deleted block. The delete fires BEFORE the
 * redock RPC resolves, so it can't be distinguished by the block's parent —
 * the floater must mark "a redock is happening" up front and `onNodeDelete`
 * must skip the delete while it's set. See issue #1662.
 *
 * Scope is the floater's renderer (the floater renders both
 * `floating-pane-workspace.tsx` and its pane's `tabcontent.tsx`, so they share
 * this module). A time-boxed window (auto-expiring) guarantees the guard can
 * never get stuck and permanently swallow a genuine pane close.
 */

let guardUntilMs = 0;

/**
 * Open the guard for `graceMs`. Called at the very start of a redock attempt
 * (before any node removal / `onNodeDelete` can fire). The grace window spans
 * the redock RPC plus the floater's close + node teardown.
 */
export function beginRedockGuard(graceMs = 5000): void {
    const until = performance.now() + graceMs;
    if (until > guardUntilMs) guardUntilMs = until;
}

/** Close the guard immediately — called when a redock attempt resolved to NO
 * redock (cursor over desktop / unresolved target), so a later genuine pane
 * close in this still-open floater isn't wrongly suppressed. */
export function endRedockGuard(): void {
    guardUntilMs = 0;
}

/** True while a redock is in flight in this renderer. */
export function isRedockGuardActive(): boolean {
    return performance.now() < guardUntilMs;
}
