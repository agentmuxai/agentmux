// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { applyFilter, badgeText, isUnread, panelMessages, topSenders, unreadCount } from "./messages-select";

let n = 0;
const jekt = (over: Partial<JektMessageNode> = {}): JektMessageNode => ({
    type: "jekt_message",
    id: `j${++n}`,
    from: "agent2",
    to: "agent1",
    message: "hello",
    raw: "",
    tier: "coord",
    deliveryTier: "host",
    trust: "host-verified",
    msgId: `m${n}`,
    priority: "normal",
    direction: "incoming",
    timestamp: 10_000,
    ...over,
});
const other = (type: string): DocumentNode => ({ type, id: `o${++n}` }) as unknown as DocumentNode;

describe("panelMessages", () => {
    it("is the incoming jekts, in order, and nothing else", () => {
        const a = jekt();
        const b = jekt({ from: "agent3" });
        const out = jekt({ direction: "outgoing" });
        expect(panelMessages([other("user_message"), a, out, other("system_note"), b])).toEqual([a, b]);
    });

    it("includes a message read back from history", () => {
        const old = jekt({ replayed: true });
        expect(panelMessages([old])).toEqual([old]);
    });
});

describe("unread", () => {
    it("is what was delivered after the seen mark", () => {
        expect(isUnread(jekt({ timestamp: 10_000 }), 9_999)).toBe(true);
        expect(isUnread(jekt({ timestamp: 10_000 }), 10_000)).toBe(false);
    });

    it("counts a held jekt from when it was delivered, not when it was sent", () => {
        expect(isUnread(jekt({ timestamp: 10_000, heldForSecs: 60 }), 20_000)).toBe(true);
    });

    it("never counts a message read back from history", () => {
        expect(isUnread(jekt({ replayed: true, timestamp: Date.now() + 5_000 }), 0)).toBe(false);
    });

    it("counts a list, and caps the badge", () => {
        expect(unreadCount([jekt(), jekt({ replayed: true }), jekt()], 0)).toBe(2);
        expect(badgeText(7)).toBe("7");
        expect(badgeText(99)).toBe("99");
        expect(badgeText(100)).toBe("99+");
    });
});

describe("filters", () => {
    const a = jekt({ from: "agent2", timestamp: 1_000 });
    const b = jekt({ from: "agent3", timestamp: 20_000 });
    const c = jekt({ from: "agent2", timestamp: 30_000 });
    const all = [a, b, c];
    const ctx = { seen: 10_000, waiting: new Set([a.id]) };

    it("keeps all, the ones that need the person, the unread, or one sender's", () => {
        expect(applyFilter(all, "all", ctx)).toEqual(all);
        expect(applyFilter(all, "needs-me", ctx)).toEqual([a]);
        expect(applyFilter(all, "unread", ctx)).toEqual([b, c]);
        expect(applyFilter(all, "from:agent2", ctx)).toEqual([a, c]);
    });

    it("lists the senders most recent first, at most the limit", () => {
        expect(topSenders(all)).toEqual(["agent2", "agent3"]);
        expect(topSenders([...all, jekt({ from: "agent4" })], 2)).toEqual(["agent4", "agent2"]);
        const many = Array.from({ length: 20 }, (_, i) => jekt({ from: `sender${i}` }));
        expect(topSenders(many)).toHaveLength(12);
    });
});
