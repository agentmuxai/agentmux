// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isSignedInNotice } from "../sign-in-notice";
import type { DocumentNode } from "../types";

/** What begins a turn: a message of the person's, a jekt, or a note the pane sends on AgentMux's behalf. */
const STARTS_A_TURN = new Set<string>(["user_message", "jekt_message", "system_note"]);

/** An auth error is in the transcript for good; a sign-in is not the last word on it. */
const isAuthError = (n: DocumentNode) => n.type === "agent_error" && (n.code === 401 || n.code === 403);

/**
 * Marks the auth errors a sign-in has answered, so a "Login Again" button never sits under "✓ Login successful".
 * An error is answered by a sign-in notice after it, or one before it with no turn starting in between:
 * the failed turn's error can land after the login that already fixed it, but a turn that starts after the login
 * (a retry, a new message) begins with a message and fails for its own reason.
 */
export function markResolvedAuthErrors(nodes: DocumentNode[]): DocumentNode[] {
    const laterSignIn: boolean[] = new Array(nodes.length).fill(false);
    let seen = false;
    for (let i = nodes.length - 1; i >= 0; i--) {
        laterSignIn[i] = seen;
        seen = seen || isSignedInNotice(nodes[i]);
    }
    let answered = false;
    let changed = false;
    const out = nodes.map((n, i) => {
        if (isSignedInNotice(n)) answered = true;
        else if (STARTS_A_TURN.has(n.type)) answered = false;
        if (n.type !== "agent_error" || !isAuthError(n) || !(answered || laterSignIn[i])) return n;
        changed = true;
        return { ...n, resolved: true };
    });
    return changed ? out : nodes;
}
