// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words the working row uses for what started a turn, and for what joined
 * it (srv's `TurnTrigger`). A turn the user started needs no words: it is the
 * turn they just asked for. Anything else is named, at the turn's start and on
 * its Worked line, so a turn the user didn't ask for never passes as theirs.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.
 */

import type { TriggerKind, TurnTrigger } from "@/app/store/agent-pane-state/turn-ledger";

/** How long an external turn's lead-in shows before the row's usual text. */
export const TRIGGER_LEAD_IN_MS = 3_000;

/** Started by something other than the user. */
export function isExternalTrigger(t: TurnTrigger | null | undefined): t is TurnTrigger {
    return !!t && t.kind !== "user" && t.kind !== "broadcast" && t.kind !== "system";
}

/** The row's opening line for an external turn: "↳ jekt from AgentX". */
export function triggerLeadIn(t: TurnTrigger | null | undefined): string | null {
    if (!isExternalTrigger(t)) return null;
    switch (t.kind) {
        case "agent":
            return `↳ jekt from ${t.from ?? "another agent"}`;
        case "service":
            return `↳ ${t.from ?? "a service"}`;
        case "schedule":
            return t.from && t.from !== "cron" ? `↳ ${t.from}` : "↳ scheduled run";
        case "task":
            return `↳ ${t.from ?? "a background task finished"}`;
        default:
            return null;
    }
}

/** The Worked line's verb: "Worked", or "Worked on AgentX's jekt". */
export function workedVerb(t: TurnTrigger | null | undefined): string {
    if (!isExternalTrigger(t)) return "Worked";
    switch (t.kind) {
        case "agent":
            return t.from ? `Worked on ${t.from}'s jekt` : "Worked on a jekt";
        case "service":
            return t.from ? `Worked on ${t.from}'s notice` : "Worked on a notice";
        case "schedule":
            return "Worked on a scheduled run";
        case "task":
            return "Worked on a finished task";
        default:
            return "Worked";
    }
}

const NOUNS: Record<TriggerKind, [string, string]> = {
    user: ["your message", "your messages"],
    broadcast: ["broadcast", "broadcasts"],
    agent: ["jekt", "jekts"],
    service: ["notice", "notices"],
    schedule: ["scheduled run", "scheduled runs"],
    task: ["task", "tasks"],
    system: ["", ""],
};

/**
 * What joined the turn after it started: "+1 jekt  ·  +2 your messages". `inputs`
 * is srv's full count; past the listed ones (srv caps the list), "+N more".
 */
export function absorbedSummary(absorbed: readonly TurnTrigger[], inputs: number): string | null {
    const counts = new Map<TriggerKind, number>();
    for (const t of absorbed) {
        if (t.kind !== "system") counts.set(t.kind, (counts.get(t.kind) ?? 0) + 1);
    }
    const parts = [...counts].map(([kind, n]) => `+${n} ${NOUNS[kind][n === 1 ? 0 : 1]}`);
    const unlisted = inputs - absorbed.length;
    if (unlisted > 0) parts.push(`+${unlisted} more`);
    return parts.length ? parts.join("  ·  ") : null;
}
