// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a row arrived live, on screen, or was loaded from history: a row
 * that arrived live is held open until it scrolls off the top
 * (`documentState.heldOpenNodes`, released by the pane's scroll-off scan).
 *
 * A finished tool whose start and end landed in one stream flush is first drawn
 * already finished, with no transition to see, and a jekt has no transition at
 * all. Both carry the time they arrived (a live tool call is stamped when it
 * comes in; a jekt carries its send time, plus `HELD_FOR` when it waited for
 * an absent recipient, see JektBubble's `deliveredAt`), and a loaded
 * transcript's rows are old, so a recent stamp is the sign of a live arrival.
 */

/** How recent a row's stamp must be for the row to count as arriving live. */
export const LIVE_ARRIVAL_WINDOW_MS = 5000;

export function arrivedLive(timestamp: number | null | undefined, now: number = Date.now()): boolean {
    return timestamp != null && now - timestamp <= LIVE_ARRIVAL_WINDOW_MS;
}
