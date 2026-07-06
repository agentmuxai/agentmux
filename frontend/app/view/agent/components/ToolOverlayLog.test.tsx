// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolOverlayLog — height-FLIP transition tests
 * (ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md).
 *
 * `scrollHeight` is always 0 in jsdom (no real layout engine), so each
 * test stubs it per-render to simulate the streaming vs. terminal content
 * having different natural heights, then asserts the FLIP mechanics:
 * the element is frozen at the "from" height synchronously, then eased
 * to the "to" height on the next animation frame.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

import { ToolOverlayLog } from "./ToolOverlayLog";
import type { ToolNode } from "../types";

let reducedMotion = false;
vi.mock("@/app/store/global", () => ({
    atoms: {
        prefersReducedMotionAtom: () => reducedMotion,
    },
}));

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    reducedMotion = false;
});

const streamingNode: ToolNode = {
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "sleep 1 && echo done" },
    status: "running",
    collapsed: false,
    summary: "Bash sleep 1 && echo done",
    log: {
        open: true,
        chunks: [{ kind: "stdout", content: "line 1", timestamp: 1 }],
    },
};

const terminalNode: ToolNode = {
    ...streamingNode,
    status: "success",
    log: { open: false, chunks: streamingNode.log!.chunks },
    result: { exitCode: 0, stdout: "line 1\ndone", stderr: "" } as any,
};

/** Stub `scrollHeight` on every HTMLDivElement instance for this test. */
function stubScrollHeight(px: number) {
    return vi.spyOn(HTMLDivElement.prototype, "scrollHeight", "get").mockReturnValue(px);
}

describe("ToolOverlayLog — height-FLIP transition", () => {
    it("freezes at the previous height then eases to the new height on a branch change", async () => {
        vi.useFakeTimers();
        const heightStub = stubScrollHeight(40); // streaming-branch height
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;

        // Let the streaming-branch effect run and record 40px as the
        // "last measured height" before anything transitions.
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("");

        heightStub.mockRestore();
        stubScrollHeight(120); // terminal-branch (result view) is taller
        setNode(terminalNode);

        // Synchronously (Solid effects run synchronously with the signal
        // write that triggers them), the element is frozen at the OLD
        // height, and the transition is already armed — the CSS
        // transition must be enabled BEFORE the new height is assigned on
        // the next frame, or there's nothing for the browser to ease.
        expect(el.style.height).toBe("40px");
        expect(el.style.transition).toContain("height");

        // Only the actual height VALUE change is deferred to the next
        // animation frame (so the browser gets to paint the frozen "from"
        // state first) — that's the only part fake timers need to advance.
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("120px");

        // Transition-end cleanup (jsdom doesn't run real CSS transitions,
        // so dispatch the event manually) clears the inline overrides.
        el.dispatchEvent(new (globalThis as any).TransitionEvent("transitionend", { propertyName: "height" }));
        expect(el.style.height).toBe("");
        expect(el.style.transition).toBe("");

        vi.useRealTimers();
    });

    it("does not animate on initial mount", () => {
        stubScrollHeight(40);
        const { container } = render(() => <ToolOverlayLog node={streamingNode} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        expect(el.style.height).toBe("");
    });

    it("does not animate when the branch is unchanged (only chunks growing)", async () => {
        vi.useFakeTimers();
        stubScrollHeight(40);
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        setNode({
            ...streamingNode,
            log: {
                open: true,
                chunks: [...streamingNode.log!.chunks, { kind: "stdout", content: "line 2", timestamp: 2 }],
            },
        });
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe(""); // still "streaming" branch — no FLIP

        vi.useRealTimers();
    });

    it("does not animate when heights are equal", async () => {
        vi.useFakeTimers();
        stubScrollHeight(60);
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        setNode(terminalNode); // scrollHeight stub still returns 60 for both
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("");

        vi.useRealTimers();
    });

    it("does not animate when the user prefers reduced motion", async () => {
        reducedMotion = true;
        vi.useFakeTimers();
        const heightStub = stubScrollHeight(40);
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        heightStub.mockRestore();
        stubScrollHeight(120);
        setNode(terminalNode);
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe(""); // branch changed + heights differ, but motion is disabled

        vi.useRealTimers();
    });
});
