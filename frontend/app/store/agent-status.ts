// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One ranking of what an agent is doing, for every place that says it: the
 * Swarm line and status chip, the agent pane's working row, and a subagent's
 * status in the Swarm and in the dock. Each place gathers the facts it has,
 * asks this for the winning kind, and words that kind its own way, so an
 * agent can't read "working" in one place and "waiting for you" in another.
 *
 * Best first:
 *
 *   needs-you    a question or approval is waiting on the user, while a turn
 *                is in flight
 *   held         the agent can't go on yet: reconnecting > compacting >
 *                stopping > rate limited > launching
 *   working      a turn is in flight
 *   interrupted  the last turn was cut off (stopped, or a subagent whose
 *                parent's turn ended under it)
 *   errored      the last turn failed
 *   idle         nothing in flight
 *
 * A surface without a fact leaves it out: the Swarm knows no compaction, and
 * the dock knows no parent turn. What a surface shows for a kind, and what it
 * shows alongside it (a title, a goal, a timer), is the surface's own.
 *
 * docs/specs/PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md step 18.
 */

export type HeldReason = "reconnecting" | "compacting" | "stopping" | "rate-limited" | "launching";

/** Held reasons, best first. */
export const HELD_ORDER: readonly HeldReason[] = ["reconnecting", "compacting", "stopping", "rate-limited", "launching"];

export type AgentStatusKind = "needs-you" | "held" | "working" | "interrupted" | "errored" | "idle";

/** Status kinds, best first. */
export const STATUS_ORDER: readonly AgentStatusKind[] = ["needs-you", "held", "working", "interrupted", "errored", "idle"];

export type AgentStatus = { kind: "held"; reason: HeldReason } | { kind: Exclude<AgentStatusKind, "held"> };

export interface AgentStatusFacts {
    /** A turn is in flight. */
    live: boolean;
    /** A question or approval is waiting on the user. Counts only while
     *  `live`: a flag left behind by a crash, or by a question answered while
     *  the pane was not mounted, must not outrank anything for an idle agent
     *  (#4234). */
    needsYou?: boolean;
    /** Each reason that applies. Reconnecting and compacting can hold an
     *  agent with no turn in flight. */
    held?: Partial<Record<HeldReason, boolean>>;
    /** How the last turn ended, read only when nothing is `live`. */
    ended?: "completed" | "interrupted" | "errored" | null;
}

/** The status that wins for these facts. */
export function rankAgentStatus(facts: AgentStatusFacts): AgentStatus {
    if (facts.live && facts.needsYou) return { kind: "needs-you" };
    const reason = HELD_ORDER.find((r) => facts.held?.[r]);
    if (reason) return { kind: "held", reason };
    if (facts.live) return { kind: "working" };
    if (facts.ended === "interrupted") return { kind: "interrupted" };
    if (facts.ended === "errored") return { kind: "errored" };
    return { kind: "idle" };
}

/** The best of several statuses, e.g. one row standing for a whole dispatch. */
export function bestStatus(statuses: readonly AgentStatus[]): AgentStatus {
    let best: AgentStatus = { kind: "idle" };
    for (const s of statuses) {
        if (STATUS_ORDER.indexOf(s.kind) < STATUS_ORDER.indexOf(best.kind)) best = s;
    }
    return best;
}

/**
 * A subagent's status, from the watcher's record and, where the surface knows
 * it, whether its parent's turn is still in flight. A subagent runs inside its
 * parent's turn, so one still recorded as active after that turn ended was cut
 * off: the watcher reconciles it to abandoned only when the pane reopens
 * (SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md, open question 1).
 */
export function subagentStatus(recorded: "active" | "completed" | "abandoned", parentLive?: boolean): AgentStatus {
    const cutOff = recorded === "abandoned" || (recorded === "active" && parentLive === false);
    return rankAgentStatus({ live: recorded === "active" && !cutOff, ended: cutOff ? "interrupted" : "completed" });
}

/** Mid-turn, held, or waiting on the user, as opposed to any way of having stopped. */
export function isActiveStatus(status: AgentStatus): boolean {
    return status.kind === "needs-you" || status.kind === "held" || status.kind === "working";
}
