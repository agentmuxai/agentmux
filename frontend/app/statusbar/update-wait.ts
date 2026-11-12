// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a restart into a staged release says while it waits: who it waits for,
 * in words a person can act on, and what "Restart anyway" would cut short.
 * srv waits for agents in the middle of a turn and for nothing else
 * (`crates/srv/src/backend/update/restart.rs`), and says so for each one.
 */

import type { WaitingOn } from "@/types/rpc/WaitingOn";

/** Agents are named one by one up to this many; more are counted. */
const NAMED_AT_MOST = 3;

export interface UpdateWait {
    /** Who the restart waits for, a sentence each. */
    lines: string[];
    /** What going ahead now cuts short. */
    interrupts: string;
}

/** "Vmer", "Vmer and Deke", "Vmer, Deke and Posa". */
function listed(names: string[]): string {
    return names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** What holds a turn up, when its request is being tried again. */
function heldUpBy(why: string): string {
    return why === "network_retry" ? "a network error" : "an API error";
}

/**
 * The wait for `waitingOn` (srv's list), or for `count` agents when srv gave no
 * list. Null when there is nobody to wait for: that is not a wait, and nothing
 * may say "waiting for 0 agents".
 */
export function updateWait(waitingOn: readonly WaitingOn[] | undefined, count: number): UpdateWait | null {
    const on = waitingOn ?? [];
    const n = on.length || count;
    if (n <= 0) return null;
    const retrying = on.filter((w) => w.why !== "turn");
    if (on.length === 0 || on.length > NAMED_AT_MOST || on.some((w) => !w.agent)) {
        const lines = [n === 1 ? "Waiting for 1 agent to finish its turn." : `Waiting for ${n} agents to finish their turn.`];
        if (retrying.length > 0) {
            lines.push(
                n === 1
                    ? "It is retrying after an error."
                    : `${retrying.length} of them ${retrying.length === 1 ? "is" : "are"} retrying after an error.`,
            );
        }
        return { lines, interrupts: n === 1 ? "Restart anyway interrupts its turn." : "Restart anyway interrupts their turns." };
    }
    const working = on.filter((w) => w.why === "turn").map((w) => w.agent);
    const lines: string[] = [];
    if (working.length > 0) {
        lines.push(`Waiting for ${listed(working)} to finish ${working.length === 1 ? "its turn" : "their turn"}.`);
    }
    for (const w of retrying) lines.push(`Waiting for ${w.agent}, which is retrying after ${heldUpBy(w.why)}.`);
    return { lines, interrupts: `Restart anyway interrupts ${listed(on.map((w) => w.agent))}.` };
}
