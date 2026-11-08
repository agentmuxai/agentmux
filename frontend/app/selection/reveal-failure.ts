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
