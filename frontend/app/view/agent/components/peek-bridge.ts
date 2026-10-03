// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * peek-bridge — which row's peek panel the pointer is currently crossing to.
 *
 * A tall peek panel (one that scrolls) is meant to be entered. Between leaving
 * its row and reaching it, the pointer may pass over OTHER rows, and each of
 * those opens its own peek after the enter delay, on top of the one the user is
 * heading for. So while a panel is bridging (its grace period after the row's
 * `mouseleave`, which always ends within HOVER_BRIDGE_MAX_MS), `useNodePeek`
 * holds off opening any other row's peek. When the bridge ends without the
 * pointer arriving, a row that is still hovered opens its peek then.
 *
 * One pointer, so one bridge at a time: a module-level signal.
 */

import { createSignal } from "solid-js";

const [bridgingRow, setBridgingRow] = createSignal<HTMLElement | null>(null);

/** The row whose peek panel is being crossed to, or null. */
export const peekBridgeRow = bridgingRow;

/** The pointer is crossing to (or is on) this row's peek panel. */
export function beginPeekBridge(row: HTMLElement | undefined): void {
    if (row) setBridgingRow(row);
}

/** That crossing is over, if it was this row's. */
export function endPeekBridge(row: HTMLElement | undefined): void {
    if (row && bridgingRow() === row) setBridgingRow(null);
}
