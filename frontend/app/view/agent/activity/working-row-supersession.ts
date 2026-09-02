// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Should the "Working…" row stand down because the ActivityDock is already
 * showing this work?
 *
 * Promotion (tool-adapter.ts) is the point at which a tool call stops being
 * "the pane is blocked on this" and becomes "this is running in the
 * background, tracked over there, with its own live countdown". That is the
 * entire purpose of auto-backgrounding a long-running shell — so continuing
 * to render `Working…` on top of it asserts the pane is busy waiting when
 * the whole point was that it no longer is. Two rows for one fact, and the
 * louder one is the wrong one.
 *
 * Deliberately narrow: this suppresses ONLY the plain "a promoted tool is
 * running and nothing else is happening" case. Every state the dock cannot
 * express keeps the row — see the individual guards below. The dock speaks
 * in tools and elapsed time; anything outside that vocabulary would be lost,
 * not relocated, if the row went away.
 *
 * Pure and separately tested (working-row-supersession.test.ts) rather than
 * inlined in agent-view.tsx: the interesting part is precisely the list of
 * exceptions, and that list is worth pinning against regression.
 *
 * See SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md.
 */

import type { TurnPhase } from "@/app/store/agent-pane-state/types";

export interface WorkingRowSupersessionInput {
    /** A Bash tool call in this pane is running AND has been promoted to a
     *  live dock row (`hasRunningPromotedTool`, tool-adapter.ts). */
    hasPromotedTool: boolean;
    /** The pane is still starting up / authenticating. */
    showingLaunchActivity: boolean;
    turnPhase: TurnPhase;
    /** Non-null while a context compaction is in flight. */
    compacting: unknown;
    /** Non-null while the stream is reconnecting. */
    reconnecting: unknown;
}

export function workingRowSupersededByDock(input: WorkingRowSupersessionInput): boolean {
    // Nothing in the dock to defer to.
    if (!input.hasPromotedTool) return false;

    // The pane isn't up yet; the dock says nothing about launch.
    if (input.showingLaunchActivity) return false;

    // "Stopping…" is about the turn, not the tool.
    if (input.turnPhase.kind === "Interrupting") return false;

    // Rate-limited / retrying — a real condition the dock has no vocabulary
    // for, and the one the user most needs to see.
    if (input.turnPhase.kind === "Streaming" && input.turnPhase.waitingReason) return false;

    // Same reasoning: neither is a tool, so neither appears in the dock.
    if (input.compacting != null) return false;
    if (input.reconnecting != null) return false;

    return true;
}
