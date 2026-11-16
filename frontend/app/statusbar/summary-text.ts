// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words and icons of the status bar's summary (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2).
 * Kept out of the components so each sentence is a test and the literal-text ratchet counts none.
 */

import type { EndedAsk } from "@/app/notification/ended-asks";
import type { Summary, SummaryClick } from "./summary";
import type { PartId } from "./summary-kinds";

export const PART_ICON: Record<PartId, string> = {
    agents: "robot",
    asks: "circle-question",
    browser: "globe",
    vault: "vault",
    tasks: "list-check",
};

export const SUMMARY_TEXT = {
    title: "Waiting on you",
    empty: "Nothing waits on you.",
    swarm: "Open the Swarm",
    section: { asks: "Asks", browser: "Browser", vault: "Vault", tasks: "Tasks", agents: "Agents" } as Record<PartId, string>,
    anotherWindow: " (in another window)",
    ended: "Recently ended",
};

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What a part says in its tooltip: the number spelled out. */
export function partTip(id: PartId, n: number): string {
    switch (id) {
        case "agents":
            return `${plural(n, "agent is", "agents are")} running`;
        case "asks":
            return `${plural(n, "agent asks", "agents ask")} you something`;
        case "browser":
            return `${plural(n, "browser pane needs", "browser panes need")} you`;
        case "vault":
            return `${plural(n, "Vault request waits", "Vault requests wait")} on you`;
        case "tasks":
            return `${plural(n, "Kanban task needs", "Kanban tasks need")} you`;
    }
}

/** The asks that already ended, in words. */
export function endedTip(n: number): string {
    return `${plural(n, "ask", "asks")} ended in the last 2 minutes`;
}

/** The whole sentence, for the item's label and the narrow bar's badge. */
export function summaryTip(s: Summary): string {
    const shown = s.parts.filter((p) => p.count > 0);
    const text = shown.map((p) => partTip(p.id, p.count)).join(", ");
    if (s.waiting > 0) return `${text}. Click to see who`;
    if (s.ended.length > 0) return `${[text, endedTip(s.ended.length)].filter(Boolean).join(", ")}. Click to see them`;
    return `${text}. Click to open the Swarm`;
}

const clock = (ms: number): string => {
    const s = Math.max(0, Math.round(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

/** How an ask that ended ended, and how long it had waited: "answered after 1:12". */
export function endedLine(a: Pick<EndedAsk, "how" | "since" | "endedAt">): string {
    return `${a.how === "answered" ? "answered" : "closed unanswered"} after ${clock(a.endedAt - a.since)}`;
}

/** A row's button: where it goes, in a word. */
export function rowButton(click: SummaryClick, parked: boolean): string {
    switch (click.to) {
        case "pane":
            return parked ? "Open" : "Go";
        case "kanban":
            return "Open";
        default:
            return "Answer";
    }
}

/** The accessible name of a row's button. */
export function rowButtonLabel(click: SummaryClick, name: string, parked: boolean): string {
    switch (click.to) {
        case "pane":
            return `${parked ? "Open" : "Go to"} ${name}`;
        case "kanban":
            return "Open the Kanban";
        default:
            return `Answer ${name}`;
    }
}

/** The countdown of a Vault use that goes ahead on silence: "goes ahead in 0:12 unless you deny". */
export function goesAheadText(atMs: number, nowMs: number): string {
    const left = Math.max(0, Math.ceil((atMs - nowMs) / 1000));
    return `goes ahead in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")} unless you deny`;
}
