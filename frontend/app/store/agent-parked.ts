// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent that runs with no pane: closing its pane, tab or window parked it (srv marks its block). Opening
// it puts a pane in front of the person again, and where that pane comes from is decided here
// (SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md follow-up; AgentX 2026-11-06): the workspace it was in, if
// that still exists, is shown in a window with the pane focused, as Switch to existing does; only when the
// workspace is gone does the pane move into the active tab.

/** The block meta srv sets on a block whose agent runs with no pane. */
export const PARKED_META = "agent:parked";

export function isParkedMeta(meta: Record<string, unknown> | undefined): boolean {
    return meta?.[PARKED_META] === true;
}

export interface ParkedReopenDeps {
    /** The block meta of `blockId`; loads the block when this window has not got it yet. */
    blockMeta: (blockId: string) => Promise<Record<string, unknown> | undefined>;
    /** Show the block wherever it is; whether it was shown. */
    reveal: (blockId: string) => Promise<boolean>;
}

/**
 * Reopen a parked agent's pane where it was: true when its block is parked and its workspace could still show
 * it (a window is promoted on the workspace and the pane focused). False when the agent has no block, the
 * block is not parked (the caller's own path handles it), or the workspace or tab is gone.
 */
export async function reopenParkedWhereItWas(blockId: string | undefined, deps: ParkedReopenDeps): Promise<boolean> {
    if (!blockId || !isParkedMeta(await deps.blockMeta(blockId))) return false;
    return deps.reveal(blockId);
}
