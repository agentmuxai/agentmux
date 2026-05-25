// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock — panel-mode render tests.
 *
 * Covers the hover-anchor extension landed alongside
 * `SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md` §5.10:
 *   - hover-only state → overlay positioning.
 *   - pinned / running / pending_approval → in-flow positioning.
 *   - hidden state → `--hidden` class.
 *
 * Pre-existing behaviors (status-icon mapping, live-tail, etc.)
 * are exercised by the integration tests in `renderers.test.ts`
 * and the broader agent-pane fixtures.
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
        expect(panel!.classList.contains("agent-tool-panel--overlay-below")).toBe(false);
        expect(panel!.classList.contains("agent-tool-panel--overlay-above")).toBe(false);
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
        // Running tools stay in flow because their content streams
        // and we don't want the row jumping around with overlay
        // positioning while a CLI is mid-output.
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

    it("hover only (post-completion, not pinned, not running) → panel is an overlay", () => {
        // Drive the 150ms enter timer to flip `hovering` on. With
        // a completed (success/failed) tool + no pin, hover puts
        // the panel into overlay mode — direction picked from
        // `getBoundingClientRect()` + the scroll container. jsdom
        // gives zeros for both, so the direction picker falls
        // through to "below" (tie-break) which is fine for
        // asserting the class FAMILY.
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-tool-block") as HTMLElement;
            fireEvent.mouseEnter(root);
            vi.advanceTimersByTime(200);
            const panel = container.querySelector(".agent-tool-panel");
            expect(panel).not.toBeNull();
            // Exactly one of the overlay classes is present.
            const isOverlay =
                panel!.classList.contains("agent-tool-panel--overlay-below") ||
                panel!.classList.contains("agent-tool-panel--overlay-above");
            expect(isOverlay).toBe(true);
            // Not in flow simultaneously.
            expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(false);
            // Hidden modifier is gone too.
            expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("hover-mode overlay has an inline `max-height` (per-hover cap)", () => {
        // Same shape as UserMessageBlock — the cap is computed
        // from container space and set inline so the overlay's
        // own overflow-y operates inside the pane bounds.
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-tool-block") as HTMLElement;
            fireEvent.mouseEnter(root);
            vi.advanceTimersByTime(200);
            const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
            expect(panel.style.maxHeight).toMatch(/^\d+px$/);
        } finally {
            vi.useRealTimers();
        }
    });

    it("pinned panel has no inline `max-height` (in-flow uses SCSS default)", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
        expect(panel.style.maxHeight).toBe("");
    });
});
