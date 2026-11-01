// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The pane line that says the agent needs a sign-in; its answer replaces it in place. */
export const SIGN_IN_NOTICE_ID = "system_notification_sign_in";

/** The id prefix of a sign-in line that stands alone (nothing asked for it). */
const SIGNED_IN_ID_PREFIX = "system_notification_signed_in_";

/** The pane line that says a sign-in worked. */
export function isSignedInNotice(n: { type: string; id: string; content?: string }): boolean {
    if (n.type !== "markdown") return false;
    return n.id.startsWith(SIGNED_IN_ID_PREFIX) || (n.id === SIGN_IN_NOTICE_ID && (n.content ?? "").startsWith("✓"));
}

type Say = (text: string, style: "info" | "warning" | "success", id: string, replaces: boolean) => void;

/**
 * The pane's "you need to sign in" line and its answer. The answer replaces the line, so the pane
 * never shows "needs you to sign in" over a working agent: a login answers it with "Logged in as ...",
 * and binding an account the user already has (the Bind button, the account picker) with
 * "Signed in with ...". A bind says nothing when the pane never asked for a sign-in (a plain account switch).
 */
export function createSignInNotice(say: Say) {
    // The line is in the pane, whatever it says now.
    let shown = false;
    // The line still asks for a sign-in.
    let asking = false;
    const put = (text: string, style: "info" | "warning" | "success") => {
        say(text, style, SIGN_IN_NOTICE_ID, shown);
        shown = true;
    };
    return {
        /** The launch flow says the agent needs a sign-in. */
        needed: (text: string, style: "info" | "warning" = "info") => {
            asking = true;
            put(text, style);
        },
        /** A login finished: its line replaces the one that asked, or stands alone when nothing asked. */
        answered: (text: string) => {
            if (asking) {
                asking = false;
                put(text, "success");
            } else {
                say(text, "success", `${SIGNED_IN_ID_PREFIX}${Date.now()}_${Math.random().toString(36).slice(2, 8)}`, false);
            }
        },
        /** An account was bound. */
        bound: (label: string) => {
            if (!asking) return;
            asking = false;
            put(`Signed in with **${label}**`, "success");
        },
    };
}
