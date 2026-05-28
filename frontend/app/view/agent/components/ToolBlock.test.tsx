// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock — panel-mode render tests.
 *
 * Post-SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28: hover is no longer a
 * trigger for expansion. Tests assert the two surviving render modes
 * (`--flow` when pinned or auto-expanded, `--hidden` otherwise) and the
 * negative case that `mouseenter` does not flip the panel open. The
 * older hover-anchor overlay variants (`--overlay-above/below`) were
 * removed alongside the hover trigger.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ToolBlock } from "./ToolBlock";
import type { ToolNode } from "../types";

afterEach(() => {
    cleanup();
});

const baseTool: ToolNode = {
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "ls" },
    status: "success",
    collapsed: true,
    summary: "Bash ls",
};

describe("ToolBlock — panel mode", () => {
    it("collapsed (default success) → panel has `--hidden` class", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(false);
    });

    it("pinned → panel is in-flow (`--flow`)", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
    });

    it("running (auto-expanded) → panel is in-flow (`--flow`)", () => {
        const running: ToolNode = { ...baseTool, status: "running" };
        const { container } = render(() => (
            <ToolBlock node={running} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    it("pending_approval (auto-expanded) → panel is in-flow", () => {
        const pending: ToolNode = { ...baseTool, status: "pending_approval" };
        const { container } = render(() => (
            <ToolBlock node={pending} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    // ── Hover is not a trigger ────────────────────────────────────────
    // Asserts the core invariant of SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28:
    // hovering a completed tool row produces zero visible state change.
    // No expansion, no overlay class, no inline max-height.
    it("mouseenter on a completed tool does NOT expand the panel", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-tool-block") as HTMLElement;
            fireEvent.mouseEnter(root);
            // Wait well past the old 150ms hover-enter delay so we
            // catch a regression where the hover timer creeps back in.
            vi.advanceTimersByTime(1000);
            const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
            expect(panel.classList.contains("agent-tool-panel--hidden")).toBe(true);
            expect(panel.classList.contains("agent-tool-panel--flow")).toBe(false);
            expect(panel.style.maxHeight).toBe("");
        } finally {
            vi.useRealTimers();
        }
    });

    it("mouseenter on a running tool keeps the panel in flow (no change)", () => {
        // Auto-expand keeps the panel open for running tools regardless
        // of hover state. Verifying the in-flow render survives the
        // mouseenter event guards against an over-zealous hover-trigger
        // removal that would also drop the auto-expand path.
        const running: ToolNode = { ...baseTool, status: "running" };
        const { container } = render(() => (
            <ToolBlock node={running} pinned={false} onTogglePin={() => {}} />
        ));
        const root = container.querySelector(".agent-tool-block") as HTMLElement;
        fireEvent.mouseEnter(root);
        const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
        expect(panel.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    it("pinned panel has no inline `max-height`", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
        expect(panel.style.maxHeight).toBe("");
    });
});
