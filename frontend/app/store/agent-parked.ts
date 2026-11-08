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

/** What showing the restored pane did. The pane is back in its tab either way (`attach` did that): this is
 *  about getting it on screen. */
export type ParkedReveal =
    /** The block was shown (or another window was told to show it, and did). */
    | { ok: true }
    /** A newer request took over the reveal; that one shows the pane, and says why itself if it cannot. */
    | { ok: false; superseded: true }
    /** Its workspace is open in no window and a window could not be opened on it. */
    | { ok: false; workspaceClosed: true }
    /** Anything else, in the reveal's own words: the pane could not be brought on screen. */
    | { ok: false; failure: string };

export interface ParkedReopenDeps {
    /** The block meta of `blockId`; loads the block when this window has not got it yet. */
    blockMeta: (blockId: string) => Promise<Record<string, unknown> | undefined>;
    /** Whether `tabId` still exists (a deleted workspace takes its tabs with it). */
    tabExists: (tabId: string) => Promise<boolean>;
    /** Put the agent's pane back in `tabId`: a parked block is in no tab, so there is nothing to show until it is. */
    attach: (tabId: string) => Promise<void>;
    /** Show the block, which `attach` just put in `tabId`; opening a window on its workspace when none shows it. */
    reveal: (blockId: string, tabId: string) => Promise<ParkedReveal>;
}

/** What the person is told when the pane is back in its workspace and that workspace could not be shown. */
export const WORKSPACE_NOT_SHOWN =
    "Its pane is back in its workspace, but that workspace could not be opened in a window.";

/** The block meta srv sets on a parked block: the tab it was parked from. */
export const PARKED_FROM_TAB_META = "agent:parkedFromTab";

/**
 * Reopen a parked agent's pane where it was. A parked block is in no tab, so it is put back in the tab it was
 * parked from (`agent:parkedFromTab`) and shown: in the window that has the workspace, or in a new one opened on
 * it, with the pane focused. True when that happened. False when the agent has no block, the block is not parked
 * (the caller's own path handles it), or the tab is gone with its workspace. Throws when the pane is back in its tab
 * but its workspace is open in no window and a window could not be opened on it: the pane is no longer parked, so
 * the active tab can't take it, and the person is told why instead of Open appearing to do nothing.
 *
 * Attaching is what makes the agent a pane again; showing it is best effort on top of that. A newer reveal taking
 * over is not a failure — it is the one showing the pane — so it says nothing; every other failure is said in the
 * reveal's own words, never as the workspace error unless that is what happened.
 */
export async function reopenParkedWhereItWas(blockId: string | undefined, deps: ParkedReopenDeps): Promise<boolean> {
    if (!blockId) return false;
    const meta = await deps.blockMeta(blockId);
    const tabId = meta?.[PARKED_FROM_TAB_META];
    if (!isParkedMeta(meta) || typeof tabId !== "string" || tabId === "" || !(await deps.tabExists(tabId)))
        return false;
    await deps.attach(tabId);
    const revealed = await deps.reveal(blockId, tabId);
    if ("failure" in revealed) throw new Error(revealed.failure);
    if ("workspaceClosed" in revealed) throw new Error(WORKSPACE_NOT_SHOWN);
    // Shown, or a newer reveal took it over: the pane is back and on its way.
    return true;
}
