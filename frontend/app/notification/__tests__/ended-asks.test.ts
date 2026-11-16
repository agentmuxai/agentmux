// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../attention-list", () => ({
    UNKNOWN_NAME: "An agent",
    attentionItems: () => [],
    attentionWho: (blockId: string) => (blockId === "b1" ? { name: "Korp", color: "#e8a33d" } : { name: "An agent" }),
}));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));
const listeners = vi.hoisted(() => new Set<(e: unknown) => void>());
vi.mock("../waiting-for-you", () => ({ waitingPanes: () => [], onWaitingEnded: (cb: (e: unknown) => void) => (listeners.add(cb), () => listeners.delete(cb)) }));

import type { AttentionItem } from "../attention-list";
import { __resetEndedAsks, ENDED_RING, ENDED_WINDOW_MS, endedAskOf, endedAsks, installEndedAsks, noteAttention, noteEndedAsk } from "../ended-asks";

afterEach(() => __resetEndedAsks());

const ended = (over: Record<string, unknown> = {}) => ({ blockId: "b1", what: "asks to approve a Bash call", kind: "own", since: 1_000, reason: "submitted" as const, ...over });

describe("endedAskOf", () => {
    it("names who asked, what, and how it ended", () => {
        expect(endedAskOf(ended(), 61_000)).toEqual({ blockId: "b1", name: "Korp", color: "#e8a33d", what: "asks to approve a Bash call", since: 1_000, endedAt: 61_000, how: "answered" });
        expect(endedAskOf(ended({ reason: "closed" }), 61_000)?.how).toBe("closed");
    });

    it("calls a request with no pane AgentMux", () => {
        expect(endedAskOf(ended({ blockId: "app:k1", kind: "consent", what: "asks for your consent" }), 61_000)?.name).toBe("AgentMux");
    });

    it("leaves out an ask answered before anyone could look for it, and a Vault ask", () => {
        expect(endedAskOf(ended({ since: 58_000 }), 61_000), "waited 3 s").toBeUndefined();
        expect(endedAskOf(ended({ since: 56_000 }), 61_000), "waited 5 s").toBeDefined();
        expect(endedAskOf(ended({ kind: "vault" }), 61_000)).toBeUndefined();
        expect(endedAskOf(ended({ since: 0 }), 61_000), "no start time").toBeUndefined();
    });
});

describe("the ring", () => {
    const ask = (n: number) => ({ blockId: `b${n}`, name: `A${n}`, what: "x", since: 0, endedAt: n * 1000, how: "answered" as const });

    it("lists the newest first, and drops what is older than two minutes", () => {
        noteEndedAsk(ask(1));
        noteEndedAsk(ask(100));
        expect(endedAsks(101_000).map((e) => e.name)).toEqual(["A100", "A1"]);
        expect(endedAsks(1_000 + ENDED_WINDOW_MS - 1).map((e) => e.name)).toEqual(["A100", "A1"]);
        expect(endedAsks(1_000 + ENDED_WINDOW_MS).map((e) => e.name)).toEqual(["A100"]);
    });

    it("keeps only the last few", () => {
        for (let n = 1; n <= ENDED_RING + 3; n++) noteEndedAsk(ask(n));
        const all = endedAsks(ENDED_RING * 1000 + 3000);
        expect(all).toHaveLength(ENDED_RING);
        expect(all[0].name).toBe(`A${ENDED_RING + 3}`);
        expect(new Set(all.map((e) => e.key)).size, "each has its own key").toBe(ENDED_RING);
    });

    it("fills as the registry says an ask ended", () => {
        installEndedAsks();
        installEndedAsks();
        expect(listeners.size, "installed once").toBe(1);
        const now = Date.now();
        listeners.forEach((cb) => cb(ended({ since: now - 30_000 })));
        expect(endedAsks(Date.now()).map((e) => e.name)).toEqual(["Korp"]);
        listeners.forEach((cb) => cb(ended({ since: now - 1_000 })));
        expect(endedAsks(Date.now()), "a brief one is not kept").toHaveLength(1);
    });
});

describe("the requester's name", () => {
    const item = (over: Partial<AttentionItem> = {}): AttentionItem => ({ blockId: "b2", name: "Alpha", color: "#d99a1e", what: "asks you a question", since: 1_000, local: true, ...over });

    it("is kept from while the ask waited, for a pane that closed before the ask ended", () => {
        noteAttention([item()], new Set(["b2"]), 10_000);
        const got = endedAskOf(ended({ blockId: "b2", what: "asks you a question", reason: "closed" }), 61_000);
        expect(got).toMatchObject({ name: "Alpha", color: "#d99a1e", how: "closed" });
    });

    it("does not override a name the window still knows", () => {
        noteAttention([item({ blockId: "b1", name: "Old" })], new Set(["b1"]), 10_000);
        expect(endedAskOf(ended(), 61_000)?.name).toBe("Korp");
    });
});

describe("an ask only srv's rows report (an agent of another window)", () => {
    const remote = (over: Partial<AttentionItem> = {}): AttentionItem => ({ blockId: "r1", name: "Zed", color: "#4aa3ff", what: "is waiting for you", since: 1_000, local: false, ...over });

    it("ends when its pane stops being listed, and is kept as answered", () => {
        noteAttention([remote()], new Set(), 20_000);
        expect(endedAsks(30_000), "still waits").toHaveLength(0);
        noteAttention([], new Set(), 31_000);
        expect(endedAsks(31_000)).toMatchObject([{ blockId: "r1", name: "Zed", what: "is waiting for you", since: 1_000, endedAt: 31_000, how: "answered" }]);
        noteAttention([], new Set(), 32_000);
        expect(endedAsks(32_000), "recorded once").toHaveLength(1);
    });

    it("leaves out one that waited under five seconds", () => {
        noteAttention([remote({ since: 28_000 })], new Set(), 28_500);
        noteAttention([], new Set(), 31_000);
        expect(endedAsks(31_000)).toHaveLength(0);
    });

    it("leaves it to the registry when this window's registry took the pane over", () => {
        noteAttention([remote()], new Set(), 20_000);
        noteAttention([], new Set(["r1"]), 31_000);
        expect(endedAsks(31_000)).toHaveLength(0);
    });

    it("does not count an ask the registry already ended as a second one", () => {
        installEndedAsks();
        noteAttention([remote({ blockId: "b1" })], new Set(), 20_000);
        listeners.forEach((cb) => cb(ended({ since: 1_000, reason: "submitted" })));
        noteAttention([], new Set(), Date.now() + 1_000);
        expect(endedAsks(Date.now() + 1_000)).toHaveLength(1);
    });

    it("is not a second ask when srv's row clears long after the registry ended it", () => {
        installEndedAsks();
        const wait = (at: number) => noteAttention([remote({ blockId: "b1", since: 1_000 })], new Set(), at);
        wait(20_000);
        const now = Date.now();
        listeners.forEach((cb) => cb(ended({ since: now - 30_000 })));
        expect(endedAsks(now + 1)).toHaveLength(1);
        wait(now + 1_000);
        wait(now + 8_000);
        noteAttention([], new Set(), now + 12_000);
        expect(endedAsks(now + 12_000), "the row clearing 12 s later adds none").toHaveLength(1);
    });

    it("is a new ask again once the pane was unlisted and waits anew", () => {
        installEndedAsks();
        const now = Date.now();
        listeners.forEach((cb) => cb(ended({ since: now - 30_000 })));
        noteAttention([remote({ blockId: "b1", since: 1_000 })], new Set(), now + 1_000);
        noteAttention([], new Set(), now + 2_000);
        noteAttention([remote({ blockId: "b1", since: now + 3_000 })], new Set(), now + 3_000);
        noteAttention([], new Set(), now + 20_000);
        expect(endedAsks(now + 20_000)).toHaveLength(2);
    });

    it("is not over while srv's row still says it waits, though Settings leaves it out of the list", () => {
        noteAttention([remote({ what: "needs you in the browser" })], new Set(), 20_000);
        noteAttention([], new Set(), 30_000, new Set(["r1"]));
        expect(endedAsks(30_000), "hidden, still waiting").toHaveLength(0);
        noteAttention([], new Set(), 40_000, new Set());
        expect(endedAsks(40_000), "answered once the row stops waiting").toMatchObject([{ blockId: "r1", what: "needs you in the browser", endedAt: 40_000 }]);
    });
});
