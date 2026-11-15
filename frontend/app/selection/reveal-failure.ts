// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The words for a reveal that did not show its pane, and the one failure among them that is not a
// timing miss: the pane's workspace is open in no window and a window could not be opened on it.
// Pure, so a caller that reports the failure itself (a parked agent's Open) and the tests can read
// them without loading the reveal's layout and selection machinery.

import { isFailure, type SelectionFailure, type SelectionResult } from "./selection-machine";

/** `remote-failed`'s detail when the pane's workspace is open in no window and a window could not be opened on it. */
export const WORKSPACE_CLOSED = "workspace-closed";

/** What the notice says for each reason a pane could not be shown. */
const FAIL_TEXT: Record<string, string> = {
    "not-found": "it is no longer open",
    "resolve-timeout": "it took too long to find",
    "tab-timeout": "its tab did not open in time",
    "layout-timeout": "its tab is still loading",
    mismatch: "it did not come to the front",
    stuck: "it did not respond in time",
    "remote-timeout": "the other window did not answer",
    "remote-failed": "the other window could not show it",
    gone: "it was just closed",
    error: "something went wrong",
};

export function failText(reason: string, detail?: string): string {
    if (reason === "remote-failed" && detail === WORKSPACE_CLOSED) {
        return "it is in a workspace no window has open, and a window could not be opened on it";
    }
    return FAIL_TEXT[reason] ?? reason;
}

/** The reveal's own words for why it did not show the pane. */
export function revealFailureText(result: SelectionFailure): string {
    return failText(result.reason, result.detail);
}

/** Whether a reveal failed for that one reason: the pane's workspace is open in no window and a window could
 *  not be opened on it. Every other failure (a slow layout, a newer request taking over) leaves the pane in a
 *  tab, and is not the same answer. */
export function isWorkspaceClosedFailure(result: SelectionResult): boolean {
    return isFailure(result) && result.reason === "remote-failed" && result.detail === WORKSPACE_CLOSED;
}

/** The reveals that ran out of time waiting for the page (its layout, its tab, its lookup): the window was slow, and
 *  nothing is wrong with the pane or its tab. */
const SLOW_SHOW_REASONS = new Set(["layout-timeout", "tab-timeout", "resolve-timeout"]);

/** Whether a reveal only ran out of time waiting for the page. For a pane whose open had already worked (its block
 *  is back in its tab), that is not a failure of the open: the pane shows when the window catches up. */
export function isSlowShow(result: SelectionResult): boolean {
    return isFailure(result) && SLOW_SHOW_REASONS.has(result.reason);
}

/** The neutral notice for a pane that is open and not yet shown because the window is slow. */
export function slowShowText(name: string): string {
    return `${name} is open. Its tab is still loading, and it will show when the window is ready.`;
}
