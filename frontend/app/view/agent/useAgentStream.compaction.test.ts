// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import type { AgentPaneEvent } from "@/app/store/agent-pane-state/types";
import type { StreamFlushQueue } from "./stream-flush-queue";
import type { DocumentNode } from "./types";
import { fillCompactionCard, pushContextCompactedNodes } from "./useAgentStream";

function fakeQueue() {
    const added: DocumentNode[] = [];
    const updated: DocumentNode[] = [];
    const queue = {
        pushNewNode: (n: DocumentNode) => added.push(n),
        pushUpdatedNode: (n: DocumentNode) => updated.push(n),
        scheduleFlush: vi.fn(),
    } as unknown as StreamFlushQueue;
    return { queue, added, updated };
}

const realEvent = {
    type: "context-compacted",
    tokensBefore: 40_697,
    tokensAfter: 1_417,
    source: "real",
    trigger: "manual",
    durationMs: 6_448,
    frameTimestamp: "2026-09-24T10:00:00Z",
} as unknown as AgentPaneEvent;

describe("the live compaction card", () => {
    it("is returned for filling when real, and filled with the next call's prompt as an in-place update", () => {
        const { queue, added, updated } = fakeQueue();
        const ids = new Set<string>();
        const card = pushContextCompactedNodes([realEvent], queue, (id) => ids.has(id), (id) => ids.add(id));
        expect(added).toHaveLength(1);
        expect(card).toMatchObject({ tokensBefore: 40_697, tokensAfter: 1_417, source: "real" });
        expect(card!.contextAfter).toBeUndefined();

        fillCompactionCard(card!, 39_490, queue);
        expect(updated).toEqual([{ ...card, contextAfter: 39_490 }]);
        expect(updated[0].id).toBe(added[0].id);
    });

    it("a heuristic card needs no filling: its after-size already is a call's prompt", () => {
        const { queue } = fakeQueue();
        const heuristic = { type: "context-compacted", tokensBefore: 60_000, tokensAfter: 4_000, source: "heuristic" } as unknown as AgentPaneEvent;
        expect(pushContextCompactedNodes([heuristic], queue, () => false, () => {})).toBeNull();
    });

    it("a card already shown (same id) isn't pushed or returned again", () => {
        const { queue, added } = fakeQueue();
        expect(pushContextCompactedNodes([realEvent], queue, () => true, () => {})).toBeNull();
        expect(added).toHaveLength(0);
    });
});
