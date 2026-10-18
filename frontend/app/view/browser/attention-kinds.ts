// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How each kind of browser-pane request looks and answers: one row per kind,
// so a new kind is a row here, not another branch in the banner.

import { siteLabel } from "./bypass";

export type AttentionLook = {
    /** Shown in the agent's attention colours: an agent (or its page) waits on the person. */
    agent: boolean;
    tone: "accent" | "danger" | "attention";
    density: "compact" | "comfortable";
    /** The head's icon (Font Awesome). */
    icon: string;
    /** The first button: its label and the decision it sends. None: nothing to say yes to. */
    yes?: string;
    answer: "done" | "approve";
    no: string;
    /** A Back button first: the way out of a navigation the pane stopped. */
    back?: boolean;
    /** Show the address the request is about. */
    showUrl?: boolean;
    /** The whole head line, for kinds whose words are all srv's fields. */
    text?: string;
};

const HAND = "fa-hand";

export function attentionLook(a: BrowserAttention): AttentionLook {
    switch (a.kind) {
        case "handoff":
            return { agent: true, tone: "attention", density: "comfortable", icon: HAND, yes: "Done", answer: "done", no: "Cancel" };
        // A page asking to leave the pane's sites keeps the quieter accent:
        // Block is the safe answer there.
        case "navigation":
            return {
                agent: false,
                tone: "accent",
                density: "compact",
                icon: HAND,
                yes: "Allow",
                answer: "approve",
                no: "Block",
                showUrl: true,
            };
        // Why a navigation didn't load; nothing waits on it
        // (REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md §6.2).
        case "notice":
            return {
                agent: false,
                tone: "accent",
                density: "compact",
                icon: "fa-circle-info",
                yes: a.allow ? "Allow" : undefined,
                answer: "approve",
                no: "Dismiss",
                back: true,
                showUrl: true,
                text: noticeText(a),
            };
        case "bypass":
            return {
                agent: true,
                tone: "danger",
                density: "comfortable",
                icon: HAND,
                yes: "Allow auto-approve",
                answer: "approve",
                no: "No",
            };
        // The page's own question (SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md §4.4):
        // the browser's own words for it.
        case "dialog":
            return a.dialogKind === "beforeunload"
                ? { agent: true, tone: "attention", density: "comfortable", icon: HAND, yes: "Leave", answer: "approve", no: "Stay" }
                : { agent: true, tone: "attention", density: "comfortable", icon: HAND, yes: "OK", answer: "approve", no: "Cancel" };
        default:
            return { agent: true, tone: "attention", density: "comfortable", icon: HAND, yes: "Approve", answer: "approve", no: "Cancel" };
    }
}

/** A notice's line: where the pane is limited to, where the page tried to go, and why it stopped. */
export function noticeText(a: BrowserAttention): string {
    const sites = (a.sites ?? []).map(siteLabel).join(", ");
    const where = a.origin ? siteLabel(a.origin) : a.url || "another address";
    const limited = sites ? `this pane is limited to ${sites}` : "this pane is limited to some sites";
    const tried = a.popup ? "open a window at" : "go to";
    const why = a.why ? ` ${a.why}` : "";
    return `Stopped: ${limited}, and the page tried to ${tried} ${where}.${why}`;
}

/** A page's dialog text, as shown: the page's words, never more than this. */
export const DIALOG_MESSAGE_MAX = 300;

export function dialogMessage(message: string | undefined): string {
    const m = (message ?? "").trim();
    return m.length > DIALOG_MESSAGE_MAX ? `${m.slice(0, DIALOG_MESSAGE_MAX)}…` : m;
}
