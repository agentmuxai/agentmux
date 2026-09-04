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
        // The height genuinely differs across the two ticks (40 -> 500) —
        // a constant stub can't distinguish "correctly gated on branch
        // staying the same" from "shouldAnimate's own zero-delta check
        // happened to block it anyway" (the growth never produced a real
        // height difference to react to either way). A prior version of
        // this test used a constant stub and stayed green even after
        // deliberately removing the branch-change gate from the source.
        vi.useFakeTimers();
        const heightStub = stubScrollHeight(40);
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        heightStub.mockRestore();
        stubScrollHeight(500);
        setNode({
            ...streamingNode,
            log: {
                open: true,
                chunks: [...streamingNode.log!.chunks, { kind: "stdout", content: "line 2", timestamp: 2 }],
            },
        });
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe(""); // still "streaming" branch — no FLIP despite a real height change

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

    it("resyncs without animating when a branch change happened entirely while the panel was hidden", () => {
        // A failed/denied/canceled tool auto-collapses the instant it
        // leaves "running" (ToolBlock.tsx autoExpanded()), so the
        // running->result branch change commonly happens while
        // content-visibility:hidden. Must show the final state directly,
        // NOT FLIP from the stale pre-collapse height (reagent P1 on
        // #1975).
        //
        // jsdom applies no real stylesheet, so the `--hidden` CLASS by
        // itself proves nothing about computed content-visibility — the
        // mechanism this migrated to (resize-contract.ts's isMeasurable)
        // reads getComputedStyle, not the class. A prior version of this
        // test asserted el.style.height === "" without this mock and
        // stayed green for the wrong reason: the scrollHeight stub simply
        // hadn't changed value yet at the point of the assertion, so
        // nothing would have animated regardless of whether hidden-
        // detection worked at all. This mock makes getComputedStyle
        // actually reflect the class, mirroring what the real stylesheet
        // does in production, so the test exercises the real gate.
        //
        // Deliberately checks el.classList directly, NOT el.closest(...) —
        // content-visibility is non-inherited, so a real getComputedStyle
        // call reports ONLY the exact queried element's own value, never an
        // ancestor's. Using closest() here would answer "hidden" for the
        // DESCENDANT too, which would make this test pass even if
        // resize-contract.ts's own isMeasurable stopped walking ancestors
        // and only checked its argument directly (confirmed: the first
        // version of this mock did exactly that, and this test stayed
        // green after deliberately breaking the ancestor walk in
        // resize-contract.ts to check).
        const realGetComputedStyle = window.getComputedStyle;
        vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) => {
            if (el.classList?.contains("agent-tool-panel--hidden")) {
                return { contentVisibility: "hidden" } as CSSStyleDeclaration;
            }
            return realGetComputedStyle(el);
        });

        stubScrollHeight(40);
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => (
            <div class="agent-tool-panel agent-tool-panel--hidden">
                <ToolOverlayLog node={node()} />
            </div>
        ));
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;

        // The branch change's real height differs sharply WHILE hidden —
        // if the hidden-gate weren't working, this is exactly the delta
        // that would produce a visible FLIP.
        stubScrollHeight(900);
        setNode(terminalNode);
        expect(el.style.height).toBe(""); // never measured/animated while hidden
        expect(el.style.transition).toBe(""); // nothing armed either — no leftover to resolve once visible
    });

    it("does not animate when a different tool node swaps into the same slot", async () => {
        // Simulates a streaming-buffer cap-advance swapping a different tool
        // node into the same <Index> slot without ever unmounting this
        // component (reagent P1 round 2 on #1975) — mirrors the
        // `prevNodeId` guard `ToolBlock.tsx` already applies for the
        // analogous slot-reuse hazard on `onHoldOpen` (PR #1317).
        vi.useFakeTimers();
        stubScrollHeight(40); // tc-1's "streaming" height
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("");

        // A different node (new id) reuses this slot, already in its
        // terminal branch, with a very different natural height.
        const otherNode: ToolNode = {
            ...terminalNode,
            id: "tc-2",
        };
        stubScrollHeight(500);
        setNode(otherNode);
        await vi.runOnlyPendingTimersAsync();
        // Must resync silently — NOT FLIP from tc-1's stale 40px baseline.
        expect(el.style.height).toBe("");

        // A genuine branch change on the NEW node (tc-2) afterwards must
        // still be able to FLIP correctly — the reset must not poison the
        // baseline for the node going forward. Its baseline height is now
        // 500px (tc-2's own terminal-branch height, recorded at the swap),
        // so this eases from 500px down to the new 120px.
        stubScrollHeight(120);
        setNode({ ...otherNode, log: { open: true, chunks: [{ kind: "stdout", content: "x", timestamp: 1 }] } });
        expect(el.style.height).toBe("500px"); // frozen "from" synchronously
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("120px"); // eased to the "to" height

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

describe("ToolOverlayLog — hides bashwrap's internal starting-chunk (2026-09-03)", () => {
    it("does not render the [bashwrap] starting system chunk while streaming", () => {
        const node: ToolNode = {
            ...streamingNode,
            log: {
                open: true,
                chunks: [
                    // The real, always-first chunk bashwrap publishes
                    // (bash_wrap.rs's one `publish_system()` call site) —
                    // reported as reading like a jarring "Thinking… ->
                    // internal debug string -> real output" transition.
                    { kind: "system", content: "[bashwrap] starting: 42 chars", timestamp: 1 },
                ],
            },
        };
        const { container } = render(() => <ToolOverlayLog node={node} />);
        expect(container.textContent).not.toContain("bashwrap");
        expect(container.querySelectorAll(".agent-tool-log-line")).toHaveLength(0);
    });

    it("shows real output immediately alongside a leading system chunk, not just eventually", () => {
        const node: ToolNode = {
            ...streamingNode,
            log: {
                open: true,
                chunks: [
                    { kind: "system", content: "[bashwrap] starting: 5 chars", timestamp: 1 },
                    { kind: "stdout", content: "real output line", timestamp: 2 },
                ],
            },
        };
        const { container } = render(() => <ToolOverlayLog node={node} />);
        expect(container.textContent).not.toContain("bashwrap");
        expect(container.textContent).toContain("real output line");
        expect(container.querySelectorAll(".agent-tool-log-line")).toHaveLength(1);
    });

    it("still shows real output once the tool completes, with no trace of the system chunk", () => {
        const node: ToolNode = {
            ...terminalNode,
            log: {
                open: false,
                chunks: [
                    { kind: "system", content: "[bashwrap] starting: 5 chars", timestamp: 1 },
                    { kind: "stdout", content: "line 1", timestamp: 2 },
                ],
            },
        };
        // Terminal + no structured result -> falls into the "chunks-final"
        // branch (still ChunkList), per the branch() logic in the component.
        const { container } = render(() => <ToolOverlayLog node={{ ...node, result: undefined }} />);
        expect(container.textContent).not.toContain("bashwrap");
        expect(container.textContent).toContain("line 1");
    });
});
