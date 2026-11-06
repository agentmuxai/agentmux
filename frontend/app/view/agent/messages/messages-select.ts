// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Messages panel lists and how it counts (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md
 * §2.2, §5.6). Everything is read from the pane's document nodes: nothing is parsed or worked out again here, so
 * resumed history and live arrivals read the same.
 */

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { deliveredAt } from "../jekts-awaiting-you";

/** An incoming jekt, in the order it arrived. The agent's own sends and echoes stay in the conversation. */
export function panelMessages(nodes: readonly DocumentNode[]): JektMessageNode[] {
    return nodes.filter((n): n is JektMessageNode => n.type === "jekt_message" && n.direction === "incoming");
}

/**
 * Not yet seen: delivered after `seen`. A message read back from stored history has no time of its own (the parser
 * stamps it with the moment it was read), so it is never unread: history is not news.
 */
export function isUnread(node: JektMessageNode, seen: number): boolean {
    return !node.replayed && deliveredAt(node) > seen;
}

export function unreadCount(messages: readonly JektMessageNode[], seen: number): number {
    return messages.filter((m) => isUnread(m, seen)).length;
}

/** The rail's badge: the count, capped. */
export const badgeText = (count: number): string => (count > 99 ? "99+" : String(count));

export type MessagesFilter = "all" | "needs-me" | "unread" | `from:${string}`;

/** The senders that have written in this pane, most recent first, at most `limit`. */
export function topSenders(messages: readonly JektMessageNode[], limit = 12): string[] {
    const seen = new Set<string>();
    const out: string[] = [];
    for (let i = messages.length - 1; i >= 0 && out.length < limit; i--) {
        const from = messages[i].from;
        if (!seen.has(from)) {
            seen.add(from);
            out.push(from);
        }
    }
    return out;
}

/** The messages a filter keeps, in their order. `waiting` are those that need the person (never cleared by being seen). */
export function applyFilter(
    messages: readonly JektMessageNode[],
    filter: MessagesFilter,
    ctx: { seen: number; waiting: ReadonlySet<string> }
): JektMessageNode[] {
    if (filter === "all") return [...messages];
    if (filter === "needs-me") return messages.filter((m) => ctx.waiting.has(m.id));
    if (filter === "unread") return messages.filter((m) => isUnread(m, ctx.seen));
    const from = filter.slice("from:".length);
    return messages.filter((m) => m.from === from);
}
