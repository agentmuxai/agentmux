// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { mayPlayFromBus, SOUNDS } from "../sounds";

// SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.8: every sound is an ask or an event.
describe("the sound classification", () => {
    it("classifies every sound; the waiting tone is the only ask", () => {
        const asks = Object.values(SOUNDS)
            .filter((d) => d.source === "ask")
            .map((d) => d.id);
        expect(Object.values(SOUNDS).every((d) => d.source === "ask" || d.source === "event")).toBe(true);
        expect(asks).toEqual(["agent.waiting.for.input"]);
    });

    it("lets an event sound play from the bus only with its pane", () => {
        const def = SOUNDS["agent.turn.complete"];
        expect(mayPlayFromBus(def, { sourceBlockId: "blk-1" })).toBe(true);
        expect(mayPlayFromBus(def, {})).toBe(false);
    });

    it("never lets an ask sound play from the bus", () => {
        expect(mayPlayFromBus(SOUNDS["agent.waiting.for.input"], { sourceBlockId: "blk-1" })).toBe(false);
    });
});
