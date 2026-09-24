// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { initialState } from "../../store/agent-document/types";
import { update } from "../../store/agent-document/reducer";
import {
    blocksRollOff,
    LIVE_FEED_DEFAULT_TURNS,
    liveFeedSupported,
    planRollOff,
    resolveLiveFeedTurns,
    splitTurns,
} from "./live-feed";
import type { DocumentNode, ShellNode, ToolNode } from "./types";

const user = (id: string): DocumentNode => ({ type: "user_message", id, message: id, timestamp: 0 });
const md = (id: string, content = "x"): DocumentNode => ({ type: "markdown", id, content }) as DocumentNode;
const tool = (id: string, extra: Partial<ToolNode> = {}): DocumentNode =>
    ({ type: "tool", id, tool: "Read", status: "success", params: {}, ...extra }) as ToolNode;
const shell = (id: string): DocumentNode => ({ type: "shell", id, status: "exited" }) as unknown as ShellNode;

/** `n` turns: u{i}, a{i} (answer). The last is the turn in flight. */
function turns(n: number): DocumentNode[] {
    const out: DocumentNode[] = [];
    for (let i = 0; i < n; i++) out.push(user(`u${i}`), md(`a${i}`));
    return out;
}
const ids = (nodes: readonly DocumentNode[]) => nodes.map((n) => n.id);
const none = new Set<string>();

describe("splitTurns", () => {
    it("starts a turn at every user_message, with a leading turn before the first", () => {
        const nodes = [md("intro"), user("u0"), md("a0"), user("u1")];
        expect(splitTurns(nodes)).toEqual([
            { start: 0, end: 1 },
            { start: 1, end: 3 },
            { start: 3, end: 4 },
        ]);
        expect(splitTurns([])).toEqual([]);
    });
});

describe("planRollOff", () => {
    it("keeps the turn in flight plus the newest K finished turns while pinned", () => {
        const nodes = turns(8); // 7 finished + 1 in flight
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true })!;
        expect(plan.turns).toBe(4);
        expect(plan.ranges).toEqual([{ start: 0, end: 8, turns: 4 }]);
    });

    it("does nothing when there is nothing beyond K", () => {
        expect(planRollOff(turns(4), { keepTurns: 3, visibleIds: none, pinned: true })).toBeNull();
        expect(planRollOff(turns(1), { keepTurns: 1, visibleIds: none, pinned: true })).toBeNull();
    });

    it("keeps any turn with a node on screen", () => {
        const nodes = turns(8);
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: new Set(["a1"]), pinned: true })!;
        // turn 1 (u1,a1) stays; 0, 2, 3 go
        expect(plan.ranges).toEqual([
            { start: 0, end: 2, turns: 1 },
            { start: 4, end: 8, turns: 2 },
        ]);
    });

    it("keeps a blocked turn without stopping the turns around it", () => {
        const nodes = turns(8);
        nodes.splice(5, 0, shell("sh")); // inside turn 2
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true })!;
        expect(plan.blockedTurns).toBe(1);
        expect(plan.turns).toBe(3);
        const kept = nodes.filter((_, i) => !plan.ranges.some((r) => i >= r.start && i < r.end));
        expect(ids(kept).slice(0, 3)).toEqual(["u2", "sh", "a2"]);
    });

    it("rolls off a turn with an answered question like any other", () => {
        const nodes = turns(8);
        nodes.splice(1, 0, tool("ask", { tool: "Other", answerText: "A", questionText: "Q?" }));
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true })!;
        expect(plan.blockedTurns).toBe(0);
        expect(plan.ranges[0].start).toBe(0);
    });

    it("keeps a turn with a node still in progress", () => {
        const nodes = turns(8);
        nodes.splice(1, 0, tool("t-running", { status: "running" }));
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true })!;
        expect(plan.ranges[0].start).toBeGreaterThan(0);
    });

    it("while scrolled up, removes only turns below what is being read", () => {
        const nodes = turns(10);
        // Reading turn 2 (nodes 4,5); turns 3..5 are below it and older than
        // the newest 3 finished (6,7,8); turns 0,1 are above and wait.
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: new Set(["u2", "a2"]), pinned: false })!;
        expect(plan.ranges).toEqual([{ start: 6, end: 12, turns: 3 }]);
    });

    it("while scrolled up over the turn in flight, removes nothing", () => {
        const nodes = turns(10);
        expect(planRollOff(nodes, { keepTurns: 3, visibleIds: new Set(["u9"]), pinned: false })).toBeNull();
        expect(planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: false })).toBeNull();
    });

    it("caps kept finished turns by size, keeping at least one", () => {
        const big = "y".repeat(400_000);
        const nodes = [user("u0"), md("a0"), user("u1"), md("a1", big), user("u2"), md("a2", big), user("u3"), md("a3", big), user("u4")];
        const plan = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true, maxFinishedBytes: 1_000_000 })!;
        // a3 and a2 fit (~800 KB), a1 would exceed 1 MB: turns 0 and 1 go
        expect(plan.ranges).toEqual([{ start: 0, end: 4, turns: 2 }]);

        const tiny = planRollOff(nodes, { keepTurns: 3, visibleIds: none, pinned: true, maxFinishedBytes: 10 })!;
        // still keeps one finished turn (u3,a3)
        expect(tiny.ranges).toEqual([{ start: 0, end: 6, turns: 3 }]);
    });
});

describe("blocksRollOff", () => {
    it("blocks in-pane shells, not tools (answered questions included) or decorations", () => {
        expect(blocksRollOff(shell("s"))).toBe(true);
        // The answer is in the transcript as the tool's result; only its
        // styled rendering is optimistic, and those fields never clear.
        expect(blocksRollOff(tool("q", { answerText: "yes", questionText: "ok?" }))).toBe(false);
        expect(blocksRollOff(tool("t"))).toBe(false);
        expect(blocksRollOff(md("stderr-1"))).toBe(false);
        expect(blocksRollOff(user("u"))).toBe(false);
    });
});

describe("settings and providers", () => {
    it("resolves the kept-turn count", () => {
        expect(resolveLiveFeedTurns(5)).toBe(5);
        expect(resolveLiveFeedTurns(0)).toBe(LIVE_FEED_DEFAULT_TURNS);
        expect(resolveLiveFeedTurns(2.5)).toBe(LIVE_FEED_DEFAULT_TURNS);
        expect(resolveLiveFeedTurns(undefined)).toBe(LIVE_FEED_DEFAULT_TURNS);
    });

    it("rolls off only for providers whose transcript holds the user's messages", () => {
        expect(liveFeedSupported("claude-stream-json", "persistent")).toBe(true);
        // Claude under the per-turn subprocess controller (muxcode, container
        // agents) never writes the user's line to the transcript.
        expect(liveFeedSupported("claude-stream-json", "subprocess")).toBe(false);
        expect(liveFeedSupported("claude-stream-json")).toBe(false);
        expect(liveFeedSupported("gemini-json", "subprocess")).toBe(true);
        expect(liveFeedSupported("codex-json")).toBe(false);
        expect(liveFeedSupported("kimi-stream-json")).toBe(false);
        expect(liveFeedSupported("acp")).toBe(false);
        expect(liveFeedSupported(undefined)).toBe(false);
    });
});

describe("RollOff reducer command", () => {
    const loaded = (nodes: DocumentNode[]) =>
        update(initialState(), { type: "HistoryLoaded", nodes }).state;

    it("removes the planned turns and keeps the id set and index in lockstep", () => {
        const state = loaded(turns(8));
        const { state: next, events } = update(state, {
            type: "RollOff",
            keepTurns: 3,
            visibleIds: none,
            pinned: true,
        });
        expect(ids(next.nodes)).toEqual(["u4", "a4", "u5", "a5", "u6", "a6", "u7", "a7"]);
        expect([...next.nodeIdSet].sort()).toEqual(ids(next.nodes).sort());
        next.nodes.forEach((n, i) => expect(next.nodeIndexById.get(n.id)).toBe(i));
        expect(events).toEqual([
            { type: "turns-rolled-off", removedCount: 8, turns: 4, blockedTurns: 0, prefixTurns: 4, gapsBefore: [] },
        ]);
    });

    it("reports where a middle range was removed, for the gap row", () => {
        const nodes = turns(8);
        nodes.splice(5, 0, shell("sh")); // turn 2 is blocked
        const { events } = update(loaded(nodes), { type: "RollOff", keepTurns: 3, visibleIds: none, pinned: true });
        expect(events[0]).toMatchObject({ prefixTurns: 2, gapsBefore: ["u4"], turns: 3 });
    });

    it("is a no-op with no events when nothing may go", () => {
        const state = loaded(turns(3));
        const res = update(state, { type: "RollOff", keepTurns: 3, visibleIds: none, pinned: true });
        expect(res.state).toBe(state);
        expect(res.events).toEqual([]);
    });

    it("drops a late update for a rolled-off node instead of re-adding it", () => {
        const state = update(loaded(turns(8)), { type: "RollOff", keepTurns: 3, visibleIds: none, pinned: true }).state;
        const { state: after } = update(state, {
            type: "StreamFlush",
            newNodes: [],
            updatedNodes: [md("a0", "late")],
        } as never);
        expect(after.nodeIdSet.has("a0")).toBe(false);
        expect(ids(after.nodes)[0]).toBe("u4");
    });
});
