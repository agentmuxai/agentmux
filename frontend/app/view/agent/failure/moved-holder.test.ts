// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { markHolderMoved, takeHolderMoved } from "./moved-holder";

describe("the pane an Open here is closing", () => {
    it("is marked once, and the close's note uses the mark up", () => {
        markHolderMoved("b1", 1000);
        expect(takeHolderMoved("b1", 2000)).toBe(true);
        expect(takeHolderMoved("b1", 2000)).toBe(false);
    });

    it("is not marked for a pane nobody marked, or for one whose mark has lapsed", () => {
        expect(takeHolderMoved("never", 1000)).toBe(false);
        markHolderMoved("b2", 1000);
        expect(takeHolderMoved("b2", 1000 + 10_001)).toBe(false);
    });
});
