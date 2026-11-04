// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DOCK_LABEL_MAX_CHARS, dockTitle } from "./dock-label";

describe("dockTitle", () => {
    it("keeps the whole summary, however many words: the row's width cuts it", () => {
        expect(dockTitle("Run tests and typecheck")).toEqual({ title: "Run tests and typecheck" });
        expect(dockTitle("Wait for the v0.60.10 release to publish on GitHub")).toEqual({
            title: "Wait for the v0.60.10 release to publish on GitHub",
        });
    });

    it("counts any run of whitespace as one gap", () => {
        expect(dockTitle("  a\n  b\tc d e f  ").title).toBe("a b c d e f");
    });

    it("cuts only a runaway text, and keeps the whole of it for the tooltip", () => {
        const long = "word ".repeat(100).trim();
        const { title, fullTitle } = dockTitle(long);
        expect(title.length).toBeLessThanOrEqual(DOCK_LABEL_MAX_CHARS + 1);
        expect(title.endsWith("…")).toBe(true);
        expect(fullTitle).toBe(long);
    });
});
