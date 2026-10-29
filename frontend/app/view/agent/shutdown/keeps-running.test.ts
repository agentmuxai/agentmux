// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { keepsRunningSetting, keepsRunningTitle } from "./keeps-running-text";

describe("keepsRunningSetting", () => {
    it("is on unless the setting says off, as srv reads it", () => {
        expect(keepsRunningSetting(undefined)).toBe(true);
        expect(keepsRunningSetting(true)).toBe(true);
        expect(keepsRunningSetting("maybe")).toBe(true);
        for (const off of [false, "false", " Off ", "no", "0", 0]) expect(keepsRunningSetting(off)).toBe(false);
    });
});

describe("keepsRunningTitle", () => {
    it("names one, two or several agents", () => {
        expect(keepsRunningTitle(["KCont"])).toBe("KCont keeps running without a pane");
        expect(keepsRunningTitle(["A", "B"])).toBe("A and B keep running without a pane");
        expect(keepsRunningTitle(["A", "B", "C", "D"])).toBe("A, B and 2 more keep running without a pane");
    });
});
