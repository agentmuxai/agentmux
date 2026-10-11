// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../types";
import { useLiveFeedRollOff } from "./useLiveFeedRollOff";

const userMsg = (id: string) => ({ type: "user_message", id, message: id }) as unknown as DocumentNode;

function mount(
    opts: {
        outputFormat?: string;
        rollOff?: { prefixTurns: number; gapsBefore: string[] };
        nodes?: DocumentNode[];
        historyOffset?: number;
    } = {}
) {
    const dispatched: Array<{ type: string }> = [];
    const [doc] = createSignal<DocumentNode[]>(opts.nodes ?? [userMsg("u1"), userMsg("u2"), userMsg("keep")]);
    const [docState, setDocState] = createSignal({
        collapsedNodes: new Set<string>(),
        heldOpenNodes: new Set<string>(),
        pinnedNodes: new Set<string>(),
    });
    const paneModel = {
        document: doc,
        dispatchDoc: (cmd: { type: string }) => {
            dispatched.push(cmd);
            if (cmd.type !== "RollOff" || !opts.rollOff) return [];
            return [
                {
                    type: "turns-rolled-off",
                    removedCount: 2,
                    turns: opts.rollOff.prefixTurns,
                    blockedTurns: 0,
                    prefixTurns: opts.rollOff.prefixTurns,
                    gapsBefore: opts.rollOff.gapsBefore,
                },
            ];
        },
    };
    const root = createRoot((dispose) => {
        const feed = useLiveFeedRollOff({
            blockId: "b1",
            paneModel: paneModel as never,
            outputFormat: () => opts.outputFormat ?? "claude-stream-json",
            block: () => undefined,
            agentAtoms: () => ({ documentStateAtom: [docState, setDocState] }) as never,
            hidden: () => false,
            history: { historyOffset: () => opts.historyOffset ?? 0 },
        });
        return { feed, dispose };
    });
    return { ...root, dispatched };
}

describe("useLiveFeedRollOff", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("is on for a format whose transcript can rebuild rolled-off turns", () => {
        const on = mount();
        const off = mount({ outputFormat: "some-other-format" });
        expect(on.feed.liveFeedOn()).toBe(true);
        expect(off.feed.liveFeedOn()).toBe(false);
        on.dispose();
        off.dispose();
    });

    it("a scheduled pass dispatches one RollOff once the browser is idle", () => {
        const m = mount();
        m.feed.scheduleRollOff();
        m.feed.scheduleRollOff(); // coalesced while one is queued
        expect(m.dispatched).toEqual([]);
        vi.advanceTimersByTime(100);
        expect(m.dispatched.filter((c) => c.type === "RollOff")).toHaveLength(1);
        m.dispose();
    });

    it("only unloads tool results when the live feed is off (Codex P2 on #4126)", () => {
        const m = mount({ outputFormat: "some-other-format" });
        m.feed.scheduleRollOff();
        vi.advanceTimersByTime(2_000);
        expect(m.dispatched.map((c) => c.type)).toEqual(["UnloadToolResults"]);
        m.dispose();
    });

    it("counts rolled-off turns and records this pass's gap rows", () => {
        const m = mount({ rollOff: { prefixTurns: 2, gapsBefore: ["keep", "gone"] } });
        expect(m.feed.earlierHistoryAvailable()).toBe(false);
        m.feed.scheduleRollOff();
        vi.advanceTimersByTime(100);
        expect(m.feed.earlierTurnsKnown()).toBe(2);
        expect(m.feed.earlierHistoryAvailable()).toBe(true);
        expect([...m.feed.gapsBefore()]).toEqual(["keep", "gone"]);
        m.dispose();
    });

    // A pane opens with about its budget restored and nothing rolled off.
    // Scrolling up used to page the whole transcript in, and "Show agent
    // history" only appeared after coming back down and a pass trimmed it.
    describe("paging older turns in stops at the budget", () => {
        // About 1 MB: at the budget, but under feedOverLimits' 1.1x trigger, so
        // no roll-off pass is due — the state a freshly opened pane is in.
        const fullFeed = (): DocumentNode[] => [
            userMsg("u1"),
            { type: "markdown", id: "a1", content: "x".repeat(1_000_000) } as unknown as DocumentNode,
            userMsg("u2"),
        ];

        it("under the budget, scrolling up still pages older turns in", () => {
            const m = mount({ historyOffset: 500 });
            expect(m.feed.canPageOlder()).toBe(true);
            expect(m.feed.earlierHistoryAvailable()).toBe(false);
            m.dispose();
        });

        it("at the budget, it stops and the History row shows before any roll-off", () => {
            const m = mount({ nodes: fullFeed(), historyOffset: 500 });
            expect(m.feed.canPageOlder()).toBe(false);
            expect(m.feed.earlierHistoryAvailable()).toBe(true);
            // The count of earlier turns isn't known: they never loaded.
            expect(m.feed.earlierTurnsKnown()).toBeUndefined();
            m.dispose();
        });

        it("shows no History row when the whole transcript is already loaded", () => {
            const m = mount({ nodes: fullFeed(), historyOffset: 0 });
            expect(m.feed.earlierHistoryAvailable()).toBe(false);
            m.dispose();
        });

        it("with the live feed off, paging is unchanged", () => {
            const m = mount({ nodes: fullFeed(), historyOffset: 500, outputFormat: "some-other-format" });
            expect(m.feed.canPageOlder()).toBe(true);
            expect(m.feed.earlierHistoryAvailable()).toBe(false);
            m.dispose();
        });
    });
});
