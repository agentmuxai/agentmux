// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { dockTitle } from "./dock-label";

describe("dockTitle", () => {
    it("keeps a phrase of five words or fewer as it is", () => {
        expect(dockTitle("Run tests and typecheck")).toEqual({ title: "Run tests and typecheck" });
        expect(dockTitle("  one two three four five  ")).toEqual({ title: "one two three four five" });
    });

    it("cuts a longer one to five words and keeps the whole text for the tooltip", () => {
        expect(dockTitle("Wait for the v0.60.10 release to publish on GitHub")).toEqual({
            title: "Wait for the v0.60.10 release…",
            fullTitle: "Wait for the v0.60.10 release to publish on GitHub",
        });
    });

    it("counts any run of whitespace as one gap", () => {
        expect(dockTitle("a\n  b\tc d e f").title).toBe("a b c d e…");
    });
});
