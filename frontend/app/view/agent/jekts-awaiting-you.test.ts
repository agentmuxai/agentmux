// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { deliveredAt, dismissalKey, jektsAwaitingYou } from "./jekts-awaiting-you";

let n = 0;
const jekt = (over: Partial<JektMessageNode> = {}): JektMessageNode => ({
    type: "jekt_message",
    id: `j${++n}`,
    from: "agent2",
    to: "agent1",
    message: "do the thing",
    raw: "",
    tier: "sensitive",
    deliveryTier: "host",
    trust: "host-verified",
    msgId: `m${n}`,
    priority: "normal",
    direction: "incoming",
    timestamp: 10_000,
    escalate: "required",
    ...over,
});
const user = (): DocumentNode => ({ type: "user_message", id: `u${++n}`, message: "yes, go" }) as DocumentNode;
const text = (): DocumentNode => ({ type: "agent_message", id: `a${++n}`, text: "ok" }) as unknown as DocumentNode;
const none = new Set<string>();

describe("jektsAwaitingYou", () => {
    it("is a sensitive jekt that requires escalation, until the person next speaks", () => {
        const waiting = jekt();
        expect(jektsAwaitingYou([text(), waiting], none, 0)).toEqual([waiting]);
        expect(jektsAwaitingYou([text(), waiting, user()], none, 0)).toEqual([]);
    });

    it("keeps waiting through a turn the pane wrote itself, and ends on a Swarm broadcast copy", () => {
        const waiting = jekt();
        const turn = (over: object): DocumentNode => ({ ...user(), ...over }) as DocumentNode;
        const supervisor = turn({ supervisor: { mode: "steady", n: 1, of: 3 } });
        expect(jektsAwaitingYou([waiting, supervisor], none, 0)).toEqual([waiting]);
        expect(jektsAwaitingYou([waiting, turn({ isStartup: true })], none, 0)).toEqual([waiting]);
        expect(jektsAwaitingYou([waiting, turn({ system: true })], none, 0)).toEqual([waiting]);
        expect(jektsAwaitingYou([waiting, turn({ broadcast: { recipients: 3 } })], none, 0)).toEqual([]);
    });

    it("is not one that only informs: sensitive from a proven sender, or a lower tier", () => {
        expect(jektsAwaitingYou([jekt({ escalate: "none" })], none, 0)).toEqual([]);
        expect(jektsAwaitingYou([jekt({ escalate: undefined })], none, 0)).toEqual([]);
        expect(jektsAwaitingYou([jekt({ tier: "coord" })], none, 0)).toEqual([]);
    });

    it("is not one read back from stored history, whatever time it carries", () => {
        expect(jektsAwaitingYou([jekt({ replayed: true, timestamp: Date.now() + 5_000 })], none, 0)).toEqual([]);
    });

    it("is not the agent's own outgoing echo", () => {
        expect(jektsAwaitingYou([jekt({ direction: "outgoing" })], none, 0)).toEqual([]);
    });

    it("lists the ones after the person's last message, oldest first", () => {
        const [old, a, b] = [jekt(), jekt(), jekt()];
        expect(jektsAwaitingYou([old, user(), a, text(), b], none, 0)).toEqual([a, b]);
    });

    it("leaves out a jekt delivered before this window started: history is not a wait", () => {
        const old = jekt({ timestamp: 5_000 });
        const fresh = jekt({ timestamp: 20_000 });
        expect(jektsAwaitingYou([old, fresh], none, 10_000)).toEqual([fresh]);
    });

    it("dates a held jekt by when it was delivered, not when it was sent", () => {
        const held = jekt({ timestamp: 5_000, heldForSecs: 10 });
        expect(deliveredAt(held)).toBe(15_000);
        expect(jektsAwaitingYou([held], none, 10_000)).toEqual([held]);
    });

    it("leaves out what the person dismissed, by the sender's message id", () => {
        const a = jekt({ msgId: "keep" });
        const b = jekt({ msgId: "gone" });
        expect(jektsAwaitingYou([a, b], new Set(["gone"]), 0)).toEqual([a]);
        expect(dismissalKey(jekt({ msgId: "", id: "node-7" }))).toBe("node-7");
    });
});
