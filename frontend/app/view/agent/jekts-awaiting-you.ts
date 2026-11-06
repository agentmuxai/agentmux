// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which incoming jekts wait on the person: a `TIER=sensitive` jekt with `ESCALATE=required`, which the agent must
 * not act on until the person confirms in their own conversation
 * (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5.4). It waits from its arrival until the
 * person next sends in the pane, or dismisses it. A jekt that came before this window opened is history, not a
 * wait: replaying a resumed session must not raise a card and loop the waiting tone for old mail. A replayed jekt has
 * no time of its own (the parser stamps it with the moment it was read), so the parser marks it `replayed`.
 */

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";

/** When this window started: a jekt delivered before it is not new. */
export const WINDOW_STARTED_MS = Date.now();

/** When the jekt reached the pane: a held one waited `heldForSecs` after its send time. */
export function deliveredAt(node: JektMessageNode): number {
    return node.timestamp + (node.heldForSecs ?? 0) * 1000;
}

export function needsConfirmation(node: DocumentNode): node is JektMessageNode {
    return (
        node.type === "jekt_message" &&
        node.direction === "incoming" &&
        !node.replayed &&
        node.tier === "sensitive" &&
        node.escalate === "required"
    );
}

/** The key a dismissal is remembered by: the message id the sender's marker carries, else the node's. */
export const dismissalKey = (node: JektMessageNode): string => node.msgId || node.id;

/**
 * A turn the person sent. The pane also writes `user_message` nodes the person did not: the start-up document and
 * a Swarm Supervisor continuation. They confirm nothing. A Swarm broadcast copy is the person's words, so it counts.
 */
const isFromPerson = (node: DocumentNode): boolean =>
    node.type === "user_message" && !node.supervisor && !node.system && !node.isStartup;

/**
 * The jekts waiting on the person, oldest first: those after the person's last message in `nodes`, delivered since
 * `since`, not dismissed. Reads from the end and stops at the person's message, so a long transcript costs the
 * length of its tail.
 */
export function jektsAwaitingYou(
    nodes: readonly DocumentNode[],
    dismissed: ReadonlySet<string>,
    since: number
): JektMessageNode[] {
    const out: JektMessageNode[] = [];
    for (let i = nodes.length - 1; i >= 0; i--) {
        const node = nodes[i];
        if (isFromPerson(node)) break;
        if (needsConfirmation(node) && deliveredAt(node) >= since && !dismissed.has(dismissalKey(node))) out.push(node);
    }
    return out.reverse();
}
