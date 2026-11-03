// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cachedRuns, highlightBody } from "./embedded-highlight";

describe("highlightBody", () => {
    it("returns runs with both themes' colours that join back to the body", async () => {
        const body = "const a = 1; // one\nexport {};\n";
        const runs = await highlightBody(body, "typescript");
        expect(runs).not.toBeNull();
        expect(runs!.map((r) => r.text).join("")).toBe(body);
        const keyword = runs!.find((r) => r.text === "const")!;
        expect(Object.keys(keyword.style!).sort()).toEqual(["--shiki-dark", "--shiki-light"]);
        expect(cachedRuns(body, "typescript")).toBe(runs);
    });

    it("refuses a body whose text Shiki would change", async () => {
        expect(await highlightBody("a = 1\r\nb = 2\r\n", "python")).toBeNull();
    });

    it("refuses an unknown language", async () => {
        expect(await highlightBody("x", "no-such-language")).toBeNull();
    });

    it("skips bodies over the line cap", async () => {
        expect(await highlightBody("x\n".repeat(2001), "python")).toBeNull();
    });
});
