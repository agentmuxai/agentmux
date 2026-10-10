// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The waiting-for-you registry: one pane waits while any of its calls to
 *  action does, and srv's announcements count only in the windows they name
 *  (REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2). */

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { addEventListener as addPaneListener } from "@/app/store/agent-pane-state-store";
import type { AgentPaneEvent } from "@/app/store/agent-pane-state/types";
import {
    __resetWaitingForYou,
    applyUserAttention,
    endWaitingForYou,
    isWaitingForYou,
    startWaitingForYou,
    type UserAttention,
} from "./waiting-for-you";

let events: [string, AgentPaneEvent][] = [];
let unsubscribe = () => {};

beforeEach(() => {
    events = [];
    unsubscribe = addPaneListener((blockId, ev) => events.push([blockId, ev]));
});

afterEach(() => {
    unsubscribe();
    __resetWaitingForYou();
});

const types = () => events.map(([b, e]) => `${b}:${e.type}`);

describe("waiting for you", () => {
    it("asks once per pane while any of its calls to action waits", () => {
        startWaitingForYou("b1", "question", "Which branch?");
        startWaitingForYou("b1", "permission", "Allow Bash?");
        expect(types()).toEqual(["b1:waiting-for-input"]);
        expect(events[0][1]).toMatchObject({ question: "Which branch?" });
        endWaitingForYou("b1", "question");
        expect(isWaitingForYou("b1")).toBe(true);
        endWaitingForYou("b1", "permission");
        expect(types()).toEqual(["b1:waiting-for-input", "b1:waiting-ended"]);
        expect(isWaitingForYou("b1")).toBe(false);
        // Ending what isn't waiting changes nothing.
        endWaitingForYou("b1", "question");
        expect(events).toHaveLength(2);
    });

    it("takes srv's announcements for this window, or for every window", () => {
        const a: UserAttention = { key: "k1", block_id: "b2", kind: "browser", text: "Lark needs you: sign in", window_ids: ["w1"], active: true };
        applyUserAttention(a, "w2");
        expect(events, "another window's pane").toHaveLength(0);
        applyUserAttention(a, "w1");
        expect(types()).toEqual(["b2:waiting-for-input"]);
        applyUserAttention({ ...a, active: false }, "w1");
        expect(types()).toEqual(["b2:waiting-for-input", "b2:waiting-ended"]);

        // No pane: every window, under an app: key.
        applyUserAttention({ ...a, key: "k2", block_id: "", window_ids: [] }, "w9");
        expect(events[2]).toEqual(["app:k2", expect.objectContaining({ type: "waiting-for-input" })]);
    });
});
