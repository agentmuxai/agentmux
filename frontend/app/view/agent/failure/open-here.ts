// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Open here" from the agent picker: bring an agent that is open in another pane into this one.
 *
 * The picker binds this pane to the agent as it does for any open, and srv refuses the second pane
 * (`open_in_other_pane`). That refusal is the pane's own Move here row, which asks for a second click
 * because it stops the agent where it runs. The person already chose that in the picker, so the pane
 * is marked here, and the row runs Move here when the refusal arrives, with no second click.
 * Move here keeps the session and sends nothing (`agent.move-here`); the pane that held the agent is
 * then closed when it is in a window this one can close it in.
 */

import { findTabHolding } from "@/app/util/reveal-block";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { closeAgentTab } from "../close-agent-tab";
import { markHolderMoved } from "./moved-holder";

/** A mark that was never answered (the agent was free, so nothing refused the pane) lapses. */
const MARK_MS = 30_000;
const wanted = new Map<string, number>();
const moving = new Set<string>();

/** The pane `blockId` was opened with "Open here": its refusal is answered with Move here. */
export function requestOpenHere(blockId: string, now = Date.now()): void {
    wanted.set(blockId, now + MARK_MS);
}

/** True once for a pane marked with {@link requestOpenHere}; the mark is used up. */
export function takeOpenHere(blockId: string, now = Date.now()): boolean {
    const until = wanted.get(blockId);
    wanted.delete(blockId);
    if (until == null || until < now) return false;
    moving.add(blockId);
    return true;
}

/** True once for a pane whose Move here was run from "Open here": the pane that held the agent is to be closed. */
export function closeHolderAfterMove(blockId: string): boolean {
    return moving.delete(blockId);
}

/** Forget a mark that was never used (the pane closed, or the open failed). */
export function forgetOpenHere(blockId: string): void {
    wanted.delete(blockId);
    moving.delete(blockId);
}

/**
 * Close the pane that held the agent, the way its own close button does. False when that pane is not
 * in a tab this window has, or is already gone: it then stays, showing that the agent moved, with Move
 * here to undo it.
 */
export async function closeHolderPane(holderBlockId: string): Promise<boolean> {
    const tabId = await findTabHolding(holderBlockId);
    if (!tabId) return false;
    const layoutModel = getLayoutModelForTabById(tabId);
    const node = layoutModel?.getNodeByBlockId(holderBlockId);
    if (!layoutModel || !node) return false;
    // The agent runs in the pane the person picked it in: the close must not say it keeps running without one.
    markHolderMoved(holderBlockId);
    await closeAgentTab({ layoutModel, ownNodeId: node.id, blockId: holderBlockId });
    return true;
}
