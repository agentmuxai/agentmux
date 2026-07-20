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
import { createSignal } from "solid-js";

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

    it("heldOpen (completed, held after live completion) → panel is in-flow", () => {
        // The scroll-driven post-completion hold: a completed tool held open
        // renders expanded until its row scrolls off the top.
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={false} heldOpen={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
    });

    it("calls onHoldOpen once when a running tool completes (active→inactive)", () => {
        const onHoldOpen = vi.fn();
        const [node, setNode] = createSignal<ToolNode>({ ...baseTool, status: "running" });
        render(() => (
            <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled(); // still running
        setNode({ ...baseTool, status: "success" }); // completes live
        expect(onHoldOpen).toHaveBeenCalledTimes(1);
    });

    it("does NOT call onHoldOpen for an already-completed tool on mount (loaded history)", () => {
        const onHoldOpen = vi.fn();
        render(() => (
            <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled();
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

    // ── Command tooltip — narrower than the removed hover-to-peek system:
    // static text only (the bare command), no expansion, suppressed once
    // the panel is already expanded. See ToolBlock.tsx's header comment.
    describe("command tooltip", () => {
        it("collapsed: hovering the name shows the bare command, not the decorated summary", () => {
            const { container, unmount } = render(() => (
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
            ));
            const anchor = container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            expect(anchor).not.toBeNull();
            fireEvent.mouseEnter(anchor);
            const tip = document.body.querySelector(".agent-tool-cmd-tooltip");
            expect(tip).not.toBeNull();
            expect(tip!.textContent).toBe("ls"); // bare params.command, not "Bash ls"
            unmount();
        });

        it("expanded (pinned): hovering the name shows no tooltip — command is visible in the panel already", () => {
            const { container, unmount } = render(() => (
                <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
            ));
            const anchor = container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchor);
            expect(document.body.querySelector(".agent-tool-cmd-tooltip")).toBeNull();
            unmount();
        });

        it("a tool kind with no extractable detail (e.g. an untyped tool) shows no tooltip", () => {
            const opaque: ToolNode = { ...baseTool, tool: "Other", params: {} };
            const { container, unmount } = render(() => (
                <ToolBlock node={opaque} pinned={false} onTogglePin={() => {}} />
            ));
            const anchor = container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchor);
            expect(document.body.querySelector(".agent-tool-cmd-tooltip")).toBeNull();
            unmount();
        });

        // ToolBlock instances are reused across status transitions via
        // index-based virtualization (no remount) -- these two assert the
        // suppression is reactive to a live `disable` change on an
        // ALREADY-MOUNTED instance, not just correct on first render. A
        // naive `if (props.disable)` early-return inside Tooltip's
        // component body would commit whichever branch was true at mount
        // and never re-select — these catch that regression.
        it("reactively starts showing the tooltip once a running (auto-expanded) tool completes, without remounting", () => {
            const [node, setNode] = createSignal<ToolNode>({ ...baseTool, status: "running" });
            const { container, unmount } = render(() => (
                <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} />
            ));
            const anchorWhileRunning = container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchorWhileRunning);
            expect(document.body.querySelector(".agent-tool-cmd-tooltip")).toBeNull(); // still running, panel expanded
            setNode({ ...baseTool, status: "success" }); // completes, panel collapses
            // `disable` flipping swaps <Show>'s branch, which mounts a fresh
            // DOM node for the anchor -- re-query rather than reuse the
            // pre-transition element reference.
            const anchorAfterComplete = container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchorAfterComplete);
            const tip = document.body.querySelector(".agent-tool-cmd-tooltip");
            expect(tip).not.toBeNull();
            expect(tip!.textContent).toBe("ls");
            unmount();
        });

        it("reactively stops showing the tooltip once an already-mounted tool gets pinned open", () => {
            const [pinned, setPinned] = createSignal(false);
            const { container, unmount } = render(() => (
                <ToolBlock node={baseTool} pinned={pinned()} onTogglePin={() => {}} />
            ));
            const queryAnchor = () => container.querySelector(".agent-tool-name-tooltip-anchor") as HTMLElement;
            fireEvent.mouseEnter(queryAnchor());
            expect(document.body.querySelector(".agent-tool-cmd-tooltip")).not.toBeNull();
            fireEvent.mouseLeave(queryAnchor());
            setPinned(true); // user clicks to pin the panel open
            // `disable` flipping swaps <Show>'s branch, which mounts a fresh
            // DOM node for the anchor -- re-query rather than reuse the
            // pre-transition element reference (a stale reference would make
            // this assertion pass vacuously against a detached node).
            fireEvent.mouseEnter(queryAnchor());
            expect(document.body.querySelector(".agent-tool-cmd-tooltip")).toBeNull();
            unmount();
        });
    });

    // ── Result-pill one-shot fade-in (reagent P2 on PR #1975) ────────────
    // The fade-in must be a one-shot class added by a `ref` callback at
    // genuine mount time, NOT a persistent CSS `animation:` on the
    // selector — the pill's `display` also toggles via an unrelated
    // `@container` width breakpoint (_responsive.scss), and a persistent
    // animation would replay on every resize across that breakpoint even
    // for an already-completed, unrelated tool call.
    describe("result-pill one-shot fade-in", () => {
        const withResult: ToolNode = {
            ...baseTool,
            result: { exitCode: 0 } as any,
        };

        it("adds the one-shot class on mount and removes it after animationend", async () => {
            const { container } = render(() => (
                <ToolBlock node={withResult} pinned={false} onTogglePin={() => {}} />
            ));
            const pill = container.querySelector(".agent-tool-result-pill") as HTMLElement;
            expect(pill).not.toBeNull();

            // The class is added a microtask after mount — deferred past
            // Solid's own dynamic `class={...}` effect, which sets
            // `className` wholesale and would otherwise clobber a
            // classList mutation made synchronously in the ref (see
            // `fadeInOnMount`'s doc comment in ToolBlock.tsx).
            await Promise.resolve();
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(true);

            pill.dispatchEvent(new Event("animationend"));
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);
        });

        it("does not re-add the class on an unrelated re-render (only a genuine remount)", async () => {
            // Regression guard for the actual bug: re-rendering with the
            // SAME resultPill (simulating a resize-driven display toggle,
            // not a real remount) must not re-add the one-shot class,
            // since the ref callback only fires on genuine DOM insertion.
            const [node, setNode] = createSignal<ToolNode>(withResult);
            const { container } = render(() => <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} />);
            const pill = container.querySelector(".agent-tool-result-pill") as HTMLElement;
            await Promise.resolve();
            pill.dispatchEvent(new Event("animationend"));
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);

            // Same status/result, just a new object reference (mirrors an
            // unrelated parent re-render) — the pill is NOT unmounted by
            // Solid (same <Show> branch stays true), so the ref never
            // re-fires and the class must stay off.
            setNode({ ...withResult });
            await Promise.resolve();
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);
        });
    });
});
