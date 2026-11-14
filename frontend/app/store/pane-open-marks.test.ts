// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { notePaneOpen, takePaneOpenTiming } from "./pane-open-marks";

describe("pane-open-marks", () => {
    it("gives the window's part of an open, split at the block's creation", () => {
        notePaneOpen("b1", 1000, 1041.4);

        expect(takePaneOpenTiming("b1", 1059.2)).toBe(" [pane-open:fe] create_block=41ms mount=18ms total=59ms");
    });

    it("answers once, and nothing for a pane this window did not just open", () => {
        notePaneOpen("b2", 0, 10);

        expect(takePaneOpenTiming("restored", 20)).toBe("");
        expect(takePaneOpenTiming("b2", 20)).not.toBe("");
        expect(takePaneOpenTiming("b2", 30)).toBe("");
    });

    it("drops the oldest open when nobody asks about them", () => {
        for (let i = 0; i < 40; i++) notePaneOpen(`n${i}`, 0, 1);

        expect(takePaneOpenTiming("n0", 2)).toBe("");
        expect(takePaneOpenTiming("n39", 2)).not.toBe("");
    });
});
