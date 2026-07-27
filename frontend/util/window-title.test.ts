// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatWindowTitle, resolveFloatingPaneName, resolveWindowName } from "./window-title";

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

    it("falls through the unrenamed bootstrap workspace name to positional — window 1 must not depend on load timing", () => {
        expect(
            resolveWindowName({
                displayName: undefined,
                workspaceName: "Starter workspace",
                indexInOpenWindows: 0,
            }),
        ).toBe("Window 1");
    });

    it("still honors a real workspace name a user happened to rename to the bootstrap default's exact text", () => {
        // Documented trade-off: a workspace deliberately renamed to the
        // literal "Starter workspace" is indistinguishable from the
        // never-renamed bootstrap default and also falls through — no
        // schema flag exists to tell them apart. Low-risk edge case.
        expect(
            resolveWindowName({
                displayName: undefined,
                workspaceName: "Starter workspace",
                indexInOpenWindows: 2,
            }),
        ).toBe("Window 3");
    });

    it("uses the workspace name when it's meaningfully different from the bootstrap default", () => {
        expect(
            resolveWindowName({
                displayName: undefined,
                workspaceName: "Starter workspace 2",
                indexInOpenWindows: 0,
            }),
        ).toBe("Starter workspace 2");
    });
});

describe("resolveFloatingPaneName", () => {
    it("uses block view label when set", () => {
        expect(
            resolveFloatingPaneName({
                blockViewLabel: "Agent",
                workspaceName: "Pulse",
                indexInOpenPanes: 0,
            }),
        ).toBe("Agent");
    });

    it("falls through to workspace name when view label is missing", () => {
        expect(
            resolveFloatingPaneName({
                blockViewLabel: undefined,
                workspaceName: "Pulse",
                indexInOpenPanes: 0,
            }),
        ).toBe("Pulse");
    });

    it("falls through the unrenamed bootstrap workspace name to positional", () => {
        expect(
            resolveFloatingPaneName({
                blockViewLabel: undefined,
                workspaceName: "Starter workspace",
                indexInOpenPanes: 1,
            }),
        ).toBe("Pane 2");
    });

    it("falls through to positional when both are missing", () => {
        expect(
            resolveFloatingPaneName({
                blockViewLabel: "",
                workspaceName: "",
                indexInOpenPanes: 0,
            }),
        ).toBe("Pane 1");
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
