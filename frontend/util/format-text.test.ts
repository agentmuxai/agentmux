// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { abbreviateText } from "./format-text";

describe("abbreviateText", () => {
    it("returns the string unchanged when it already fits", () => {
        expect(abbreviateText("short", 40)).toBe("short");
        expect(abbreviateText("", 10)).toBe("");
    });

    it("truncates a plain string from the end with a real ellipsis", () => {
        expect(abbreviateText("a very long message that overflows", 10)).toBe("a very lo…");
    });

    it("right-truncates even a slash-containing string when pathAware is not set (default false)", () => {
        // A URL/expression containing "/" must NOT be silently left-truncated
        // unless the caller opts in — see this file's own doc comment for why.
        expect(abbreviateText("https://api.example.com/v1/users", 15)).toBe("https://api.ex…");
    });

    it("left-truncates a path-like string to preserve the filename when pathAware is true", () => {
        expect(abbreviateText("/a/b/c/file.ts", 8, { pathAware: true })).toBe("…file.ts");
        expect(abbreviateText(String.raw`C:\proj\file.ts`, 9, { pathAware: true })).toBe(String.raw`…\file.ts`);
    });

    it("still right-truncates a non-path string even with pathAware: true (reagent P1, PR #2387)", () => {
        // pathAware only enables the / \ check — it must not force every
        // input through left-truncation. AgentFooter.tsx's real inputs
        // include non-path tool args (grep patterns, Bash flags, etc.).
        expect(abbreviateText("--verbose --no-color --max-count=100", 15, { pathAware: true }))
            .toBe("--verbose --no…");
    });

    it("uses the real ellipsis character, not three literal dots", () => {
        const result = abbreviateText("a very long message that overflows", 10);
        expect(result).toContain("…");
        expect(result).not.toContain("...");
    });
});
