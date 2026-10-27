// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's "you need to sign in" line and its answer. A login answers it with "Logged in as ...";
 * binding an account the user already has (the Bind button, the account picker) answers it with
 * "Signed in with ...", so the line never stands over a working agent. Posts nothing for a bind
 * when the pane never said sign-in was needed (a plain account switch).
 */
export function createSignInNotice(post: (text: string, style: "success") => void) {
    let posted = false;
    return {
        /** A line the launch flow posted: remembers whether it was the sign-in one. */
        seen: (kind?: "sign-in") => {
            if (kind === "sign-in") posted = true;
        },
        /** A login finished: it posts its own line. */
        answered: () => {
            posted = false;
        },
        /** An account was bound. */
        bound: (label: string) => {
            if (!posted) return;
            posted = false;
            post(`Signed in with **${label}**`, "success");
        },
    };
}
