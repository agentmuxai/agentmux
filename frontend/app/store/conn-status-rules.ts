// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a pane does with its connection's status (conn-status.ts). srv says
// how a connection is: connecting while a login may be under way (an SSH
// terminal's ssh at a password prompt), connected once logged in, and
// disconnected or error once it is gone (crates/srv/src/backend/remote/link.rs).

/** Whether a pane's connection bar (`ConnStatusOverlay`) shows: when
 *  something went wrong, or the connection it had is gone. Not while
 *  connecting: an SSH terminal shows that itself, ssh's prompts included, at
 *  the top of the pane, where the bar covered them. And not before anything
 *  has happened (a new pane, a Files pane before its first listing). */
export function connBarShows(cs: ConnStatus | null | undefined): boolean {
    if (cs == null) return false;
    return cs.status === "error" || (cs.status === "disconnected" && !!cs.hasconnected);
}

/** Whether a terminal re-syncs its process when its connection's status
 *  changes from `prev` to `next`: when the connection comes up (a finished
 *  pane starts again over it, after a Reconnect or another pane's login), or
 *  the pane moved to another connection. Never when it goes: that restarted
 *  ssh at once, a loop of attempts against a host that is down (#4513). */
export function resyncsOnConnChange(
    prev: ConnStatus | null | undefined,
    next: ConnStatus | null | undefined
): boolean {
    if (prev?.connection !== next?.connection) return true;
    return next?.status === "connected" && prev?.status !== "connected";
}
