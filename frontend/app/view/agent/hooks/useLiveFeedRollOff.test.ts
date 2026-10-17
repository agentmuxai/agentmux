// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode } from "../types";
import { LIVE_FEED_MAX_FINISHED_BYTES } from "../live-feed";
import { BUDGET_PASS_MS, HIDDEN_HOLD_RELEASE_MS, IN_FLIGHT_TEXT_BUDGET, useLiveFeedRollOff } from "./useLiveFeedRollOff";

const userMsg = (id: string) => ({ type: "user_message", id, message: id }) as unknown as DocumentNode;

function mount(
    opts: {
        outputFormat?: string;
        rollOff?: { prefixTurns: number; gapsBefore: string[] };
        nodes?: DocumentNode[];
        historyOffset?: number;
        hidden?: () => boolean;
        held?: string[];
    } = {}
) {
    const dispatched: Array<{ type: string }> = [];
    const [doc] = createSignal<DocumentNode[]>(opts.nodes ?? [userMsg("u1"), userMsg("u2"), userMsg("keep")]);
    const [docState, setDocState] = createSignal({
        collapsedNodes: new Set<string>(),
        heldOpenNodes: new Set<string>(opts.held ?? []),
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
            hidden: opts.hidden ?? (() => false),
            history: { historyOffset: () => opts.historyOffset ?? 0 },
        });
        return { feed, dispose };
    });
    return { ...root, dispatched, docState };
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

// SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md §2.3, §2.6.
describe("useLiveFeedRollOff — holds in hidden panes, and passes while over budget", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    // REPORT_AGENT_PANE_HELD_TOOLS_COLLAPSE_ON_RETURN_RCA_2026_10_17.md: leaving
    // a pane must not collapse what was open; only the held rows' text may go.
    const unloadKeeps = (m: ReturnType<typeof mount>, from: number): Set<string>[] =>
        m.dispatched
            .slice(from)
            .filter((c) => c.type === "UnloadToolResults")
            .map((c) => (c as unknown as { keepIds: Set<string> }).keepIds);

    it("keeps held-open rows however long the pane is hidden; after HIDDEN_HOLD_RELEASE_MS their results may unload", () => {
        const [hidden, setHidden] = createSignal(false);
        const m = mount({ hidden, held: ["t1"] });
        setHidden(true);
        vi.advanceTimersByTime(HIDDEN_HOLD_RELEASE_MS - 1);
        // The pass on hiding still keeps the held row's result.
        expect(unloadKeeps(m, 0).every((k) => k.has("t1"))).toBe(true);
        const before = m.dispatched.length;
        vi.advanceTimersByTime(1 + 2_000);
        vi.advanceTimersByTime(HIDDEN_HOLD_RELEASE_MS * 10);
        // Still held: the row stays open when the pane is shown again.
        expect(m.docState().heldOpenNodes.has("t1")).toBe(true);
        // But a pass now lets its text go.
        const after = unloadKeeps(m, before);
        expect(after.length).toBeGreaterThan(0);
        expect(after.some((k) => !k.has("t1"))).toBe(true);
        m.dispose();
    });

    it("keeps held results loaded when the pane is shown again before the grace ends", () => {
        const [hidden, setHidden] = createSignal(false);
        const m = mount({ hidden, held: ["t1"] });
        setHidden(true);
        vi.advanceTimersByTime(HIDDEN_HOLD_RELEASE_MS / 2);
        setHidden(false);
        const before = m.dispatched.length;
        vi.advanceTimersByTime(HIDDEN_HOLD_RELEASE_MS * 2);
        expect(m.docState().heldOpenNodes.has("t1")).toBe(true);
        expect(unloadKeeps(m, before).every((k) => k.has("t1"))).toBe(true);
        m.dispose();
    });

    it("still unloads a hidden pane's finished tools that weren't held", () => {
        const [hidden, setHidden] = createSignal(false);
        const m = mount({ hidden, held: ["t1"] });
        setHidden(true);
        vi.advanceTimersByTime(2_000);
        const keeps = unloadKeeps(m, 0);
        expect(keeps.length).toBeGreaterThan(0);
        // Only pinned and held ids are ever kept: "t2" (finished, not held) is unloadable.
        expect(keeps.every((k) => !k.has("t2"))).toBe(true);
        m.dispose();
    });

    it("runs a pass every BUDGET_PASS_MS while the pane holds more text than its budget", () => {
        const huge = "x".repeat(LIVE_FEED_MAX_FINISHED_BYTES + IN_FLIGHT_TEXT_BUDGET + 1_000);
        const nodes = [userMsg("u1"), { type: "markdown", id: "m1", content: huge } as unknown as DocumentNode];
        const m = mount({ nodes });
        vi.advanceTimersByTime(2_000);
        const first = m.dispatched.filter((c) => c.type === "UnloadToolResults").length;
        vi.advanceTimersByTime(BUDGET_PASS_MS * 3 + 2_000);
        const later = m.dispatched.filter((c) => c.type === "UnloadToolResults").length;
        expect(later - first).toBeGreaterThanOrEqual(3);
        m.dispose();
    });

    it("runs the periodic pass with the live feed off too, without rolling turns off", () => {
        const huge = "x".repeat(LIVE_FEED_MAX_FINISHED_BYTES + IN_FLIGHT_TEXT_BUDGET + 1_000);
        const nodes = [userMsg("u1"), { type: "markdown", id: "m1", content: huge } as unknown as DocumentNode];
        const m = mount({ nodes, outputFormat: "some-other-format" });
        expect(m.feed.liveFeedOn()).toBe(false);
        vi.advanceTimersByTime(2_000);
        const first = m.dispatched.filter((c) => c.type === "UnloadToolResults").length;
        vi.advanceTimersByTime(BUDGET_PASS_MS * 3 + 2_000);
        const later = m.dispatched.filter((c) => c.type === "UnloadToolResults").length;
        expect(later - first).toBeGreaterThanOrEqual(3);
        expect(m.dispatched.filter((c) => c.type === "RollOff")).toHaveLength(0);
        m.dispose();
    });

    it("runs the periodic pass once the turn in flight passes IN_FLIGHT_TEXT_BUDGET, well under the combined limit", () => {
        const turn = "x".repeat(IN_FLIGHT_TEXT_BUDGET + 50_000);
        const nodes = [userMsg("u1"), { type: "markdown", id: "m1", content: turn } as unknown as DocumentNode];
        const m = mount({ nodes, outputFormat: "some-other-format" });
        vi.advanceTimersByTime(2_000);
        const first = m.dispatched.filter((c) => c.type === "UnloadToolResults").length;
        vi.advanceTimersByTime(BUDGET_PASS_MS * 2 + 2_000);
        expect(m.dispatched.filter((c) => c.type === "UnloadToolResults").length - first).toBeGreaterThanOrEqual(2);
        m.dispose();
    });

    it("runs no periodic pass under budget", () => {
        const m = mount();
        vi.advanceTimersByTime(BUDGET_PASS_MS * 3);
        expect(m.dispatched.filter((c) => c.type === "UnloadToolResults")).toHaveLength(0);
        m.dispose();
    });
});
