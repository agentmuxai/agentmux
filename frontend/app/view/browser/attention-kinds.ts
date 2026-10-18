// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How each kind of browser-pane request looks and answers: one row per kind,
// so a new kind is a row here, not another branch in the banner
// (REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md §7.4).

export type AttentionLook = {
    /** Shown in the agent's attention colours: an agent (or its page) waits on the person. */
    agent: boolean;
    tone: "accent" | "danger" | "attention";
    density: "compact" | "comfortable";
    /** The first button: its label and the decision it sends. */
    yes: string;
    answer: "done" | "approve";
    no: string;
};

export function attentionLook(a: BrowserAttention): AttentionLook {
    switch (a.kind) {
        case "handoff":
            return { agent: true, tone: "attention", density: "comfortable", yes: "Done", answer: "done", no: "Cancel" };
        // A page asking to leave the pane's sites keeps the quieter accent:
        // Block is the safe answer there.
        case "navigation":
            return { agent: false, tone: "accent", density: "compact", yes: "Allow", answer: "approve", no: "Block" };
        case "bypass":
            return {
                agent: true,
                tone: "danger",
                density: "comfortable",
                yes: "Allow auto-approve",
                answer: "approve",
                no: "No",
            };
        // The page's own question (SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md §4.4):
        // the browser's own words for it.
        case "dialog":
            return a.dialogKind === "beforeunload"
                ? { agent: true, tone: "attention", density: "comfortable", yes: "Leave", answer: "approve", no: "Stay" }
                : { agent: true, tone: "attention", density: "comfortable", yes: "OK", answer: "approve", no: "Cancel" };
        default:
            return { agent: true, tone: "attention", density: "comfortable", yes: "Approve", answer: "approve", no: "Cancel" };
    }
}

/** A page's dialog text, as shown: the page's words, never more than this. */
export const DIALOG_MESSAGE_MAX = 300;

export function dialogMessage(message: string | undefined): string {
    const m = (message ?? "").trim();
    return m.length > DIALOG_MESSAGE_MAX ? `${m.slice(0, DIALOG_MESSAGE_MAX)}…` : m;
}
