// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A fallback re-delivery's card (CD2b) stands for its hidden message, whose
 * replay otherwise makes a "Memory reinjected" row of its own. Within one
 * history page the parser pairs them (`noteFallbackCard` in
 * `stream-parser.ts`); where nodes are joined (an older page, a snapshot, a
 * live batch) only the document sees both, so the reducer pairs them there.
 * docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 B1.
 */

import type { DocumentNode } from "../../view/agent/types";

const isFallbackCard = (n: DocumentNode | undefined): boolean => n?.type === "context_delivery" && n.fallback === true;

/** A node `fallbackEchoIndexes` can pair: a fallback card, or a "Memory reinjected" row. */
export function isFallbackEchoPart(n: DocumentNode): boolean {
    return n.type === "memory_reinjection" || isFallbackCard(n);
}

/**
 * The "Memory reinjected" rows among `nodes[from, to)` that a fallback card
 * stands for: a row right next to a fallback card. Only a fallback card has
 * a hidden message, and a row from before CD2b has no card, so either stays
 * when its neighbour isn't the other.
 */
export function fallbackEchoIndexes(nodes: readonly DocumentNode[], from: number, to: number): number[] {
    const out: number[] = [];
    for (let i = Math.max(0, from); i < Math.min(nodes.length, to); i++) {
        if (nodes[i].type === "memory_reinjection" && (isFallbackCard(nodes[i - 1]) || isFallbackCard(nodes[i + 1]))) {
            out.push(i);
        }
    }
    return out;
}
