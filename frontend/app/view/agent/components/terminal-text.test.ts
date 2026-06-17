// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { terminalText } from "./terminal-text";

describe("terminalText", () => {
    it("returns a raw string result as-is", () => {
        expect(terminalText("line1\nline2")).toBe("line1\nline2");
    });

    it("prefers stdout, joining stderr when both present", () => {
        expect(terminalText({ stdout: "out", stderr: "err" })).toBe("out\nerr");
        expect(terminalText({ stdout: "out" })).toBe("out");
        expect(terminalText({ stderr: "err" })).toBe("err");
    });

    it("falls back to output then content", () => {
        expect(terminalText({ output: "the output" })).toBe("the output");
        expect(terminalText({ content: "the content" })).toBe("the content");
        // stdout wins over output/content
        expect(terminalText({ stdout: "s", output: "o", content: "c" })).toBe("s");
        // output wins over content
        expect(terminalText({ output: "o", content: "c" })).toBe("o");
    });

    it("returns null for a purely structured result", () => {
        expect(terminalText({ a: 1, b: 2 })).toBeNull();
        expect(terminalText({ files: ["a", "b"] })).toBeNull();
        expect(terminalText({ exitCode: 0 })).toBeNull();
    });

    it("returns null for empty / nullish bodies (so callers show JSON)", () => {
        expect(terminalText(null)).toBeNull();
        expect(terminalText(undefined)).toBeNull();
        expect(terminalText("")).toBeNull();
        expect(terminalText({ stdout: "", stderr: "" })).toBeNull();
        expect(terminalText({ content: "" })).toBeNull();
        expect(terminalText(42)).toBeNull();
    });
});
