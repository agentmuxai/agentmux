// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../attention-list", () => ({ attentionWho: (blockId: string) => (blockId === "b1" ? { name: "Korp", color: "#e8a33d" } : { name: "An agent" }) }));
const listeners = vi.hoisted(() => new Set<(e: unknown) => void>());
vi.mock("../waiting-for-you", () => ({ onWaitingEnded: (cb: (e: unknown) => void) => (listeners.add(cb), () => listeners.delete(cb)) }));

import { __resetEndedAsks, ENDED_RING, ENDED_WINDOW_MS, endedAskOf, endedAsks, installEndedAsks, noteEndedAsk } from "../ended-asks";

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
