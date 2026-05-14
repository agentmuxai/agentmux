// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatWindowTitle, resolveWindowName } from "./window-title";

describe("resolveWindowName", () => {
    it("uses display name when set", () => {
        expect(
            resolveWindowName({
                displayName: "Debug Session",
                workspaceName: "Pulse",
                indexInOpenWindows: 0,
            }),
        ).toBe("Debug Session");
    });

    it("trims display name", () => {
        expect(
            resolveWindowName({
                displayName: "   Spaced Out   ",
                workspaceName: undefined,
                indexInOpenWindows: 0,
            }),
        ).toBe("Spaced Out");
    });

    it("falls through when display name is whitespace-only", () => {
        expect(
            resolveWindowName({
                displayName: "   ",
                workspaceName: "Pulse",
                indexInOpenWindows: 4,
            }),
        ).toBe("Pulse");
    });

    it("falls through to workspace name when display name is undefined", () => {
        expect(
            resolveWindowName({
                displayName: undefined,
                workspaceName: "Pulse",
                indexInOpenWindows: 0,
            }),
        ).toBe("Pulse");
    });

    it("falls through to workspace name when display name is null", () => {
        expect(
            resolveWindowName({
                displayName: null,
                workspaceName: "Pulse",
                indexInOpenWindows: 0,
            }),
        ).toBe("Pulse");
    });

    it("trims workspace name", () => {
        expect(
            resolveWindowName({
                displayName: "",
                workspaceName: "  Stratum  ",
                indexInOpenWindows: 0,
            }),
        ).toBe("Stratum");
    });

    it("falls through to positional when both names are missing", () => {
        expect(
            resolveWindowName({
                displayName: "",
                workspaceName: "",
                indexInOpenWindows: 0,
            }),
        ).toBe("Window 1");
    });

    it("uses 1-indexed positional", () => {
        expect(
            resolveWindowName({
                displayName: undefined,
                workspaceName: undefined,
                indexInOpenWindows: 4,
            }),
        ).toBe("Window 5");
    });
});

describe("formatWindowTitle", () => {
    it("joins all three parts when tab name is present", () => {
        expect(formatWindowTitle("Main", "Shell")).toBe("Main - Shell - AgentMux");
    });

    it("trims tab name", () => {
        expect(formatWindowTitle("Main", "  Shell  ")).toBe("Main - Shell - AgentMux");
    });

    it("omits the empty middle slot when tab name is empty", () => {
        expect(formatWindowTitle("Main", "")).toBe("Main - AgentMux");
    });

    it("omits the empty middle slot when tab name is whitespace-only", () => {
        expect(formatWindowTitle("Main", "   ")).toBe("Main - AgentMux");
    });

    it("omits the empty middle slot when tab name is undefined", () => {
        expect(formatWindowTitle("Main", undefined)).toBe("Main - AgentMux");
    });

    it("omits the empty middle slot when tab name is null", () => {
        expect(formatWindowTitle("Main", null)).toBe("Main - AgentMux");
    });

    it("preserves user names that contain the separator literal", () => {
        // Documented trade-off in the spec §8 — '-' inside a name is
        // visually ambiguous with the separator but not corrupted.
        expect(formatWindowTitle("Foo - Bar", "Logs")).toBe("Foo - Bar - Logs - AgentMux");
    });

    it("renders 64-char window names without truncating", () => {
        const longName = "A".repeat(64);
        const out = formatWindowTitle(longName, "X");
        expect(out.startsWith(longName)).toBe(true);
        expect(out).toBe(`${longName} - X - AgentMux`);
    });
});
