// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { closeHolderAfterMove, forgetOpenHere, requestOpenHere, takeOpenHere } from "./open-here";

describe("the Open here mark", () => {
    it("is taken once, then used up", () => {
        requestOpenHere("b1", 1000);
        expect(takeOpenHere("b1", 1500)).toBe(true);
        expect(takeOpenHere("b1", 1500)).toBe(false);
    });

    it("is not given to a pane that was never marked", () => {
        expect(takeOpenHere("never", 1000)).toBe(false);
    });

    it("lapses after 30 seconds, so a later real refusal in that pane still asks for its click", () => {
        requestOpenHere("b2", 1000);
        expect(takeOpenHere("b2", 1000 + 30_001)).toBe(false);
    });

    it("asks for the holder to be closed once, only after the mark was taken", () => {
        expect(closeHolderAfterMove("b3")).toBe(false);
        requestOpenHere("b3", 1000);
        takeOpenHere("b3", 1001);
        expect(closeHolderAfterMove("b3")).toBe(true);
        expect(closeHolderAfterMove("b3")).toBe(false);
    });

    it("is forgotten when the open fails", () => {
        requestOpenHere("b4", 1000);
        forgetOpenHere("b4");
        expect(takeOpenHere("b4", 1001)).toBe(false);
    });
});
