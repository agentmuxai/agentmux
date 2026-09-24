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

interface Heights {
    /** `scrollHeight` — the FLIP's own from/to measure (true content height). */
    scroll: number;
    /** `offsetHeight` — the rendered box, what magnitude gating uses (codex
     *  P2, PR #2962). jsdom's own default is 0, which shouldAnimate's
     *  fromPx<=0 guard would treat as "nothing to FLIP from". */
    offset: number;
}

/**
 * Heights as a function of what the log currently RENDERS, the way a real
 * layout engine answers — not of when the stub was installed.
 *
 * The FLIP captures its "from" height just before a branch change is patched
 * into the DOM (a pure computation, ahead of Solid's render effects), and its
 * "to" height just after. Earlier versions of these tests swapped a
 * fixed-value stub right before changing the node, which only modelled an
 * implementation that re-measured a baseline on every update; that per-update
 * re-measure was the cost removed in
 * TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md §3.4.
 *
 * `heightsFor` receives the `.agent-tool-overlay-log` element; any other div
 * reads 0.
 */
function stubHeights(heightsFor: (el: HTMLElement) => Heights) {
    const forLog = (el: HTMLElement, pick: keyof Heights) =>
        el.classList.contains("agent-tool-overlay-log") ? heightsFor(el)[pick] : 0;
    vi.spyOn(HTMLDivElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
        return forLog(this, "scroll");
    });
    vi.spyOn(HTMLDivElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
        return forLog(this, "offset");
    });
}

/** True while the log shows the streaming chunk feed rather than a result view. */
const showsChunkFeed = (el: HTMLElement): boolean => el.querySelector(".agent-tool-log-line") != null;

/** One height while the chunk feed is shown, another for the result view. */
function stubHeightsByBranch(chunkFeed: Heights, result: Heights) {
    stubHeights((el) => (showsChunkFeed(el) ? chunkFeed : result));
}

describe("ToolOverlayLog — height-FLIP transition", () => {
    it("freezes at the previous height then eases to the new height on a branch change", async () => {
        vi.useFakeTimers();
        // The result view is taller. The rendered (offset) heights must
        // genuinely differ too, not just be nonzero: shouldAnimate's
        // magnitude check needs a real gate DELTA, so scrollHeight (the
        // FLIP's own from/to) is what drives the outcome.
        stubHeightsByBranch({ scroll: 40, offset: 50 }, { scroll: 120, offset: 80 });
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;

        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("");

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
        stubHeightsByBranch({ scroll: 40, offset: 50 }, { scroll: 120, offset: 80 });
        const { container } = render(() => <ToolOverlayLog node={streamingNode} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        expect(el.style.height).toBe("");
    });

    it("does not animate when the branch is unchanged (only chunks growing)", async () => {
        // The height genuinely differs across the two ticks (40 -> 80 per
        // chunk line, by what is rendered) — a constant stub can't
        // distinguish "correctly gated on branch staying the same" from
        // "shouldAnimate's own zero-delta check happened to block it
        // anyway". A prior version of this test used a constant stub and
        // stayed green even after deliberately removing the branch-change
        // gate from the source.
        vi.useFakeTimers();
        stubHeights((el) => {
            const lines = el.querySelectorAll(".agent-tool-log-line").length;
            return { scroll: 40 * lines, offset: 50 * lines };
        });
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
        expect(el.style.height).toBe(""); // still "streaming" branch — no FLIP despite a real height change

        vi.useRealTimers();
    });

    it("does not animate when heights are equal", async () => {
        vi.useFakeTimers();
        stubHeightsByBranch({ scroll: 60, offset: 60 }, { scroll: 60, offset: 60 });
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

        // The branch change's real height differs sharply WHILE hidden —
        // if the hidden-gate weren't working, this is exactly the delta
        // that would produce a visible FLIP.
        stubHeightsByBranch({ scroll: 40, offset: 50 }, { scroll: 900, offset: 400 });
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => (
            <div class="agent-tool-panel agent-tool-panel--hidden">
                <ToolOverlayLog node={node()} />
            </div>
        ));
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;

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
        // tc-1 streaming 40px; tc-2's result view 500px; tc-2 streaming (its
        // one chunk is "x") 120px. offsetHeight (the magnitude-gating
        // measure, codex P2 PR #2962) genuinely differs across the third
        // phase's compared reads — a constant value produces a zero gate
        // delta regardless of how scrollHeight moves.
        stubHeights((el) =>
            !showsChunkFeed(el)
                ? { scroll: 500, offset: 55 }
                : el.textContent?.includes("line 1")
                  ? { scroll: 40, offset: 50 }
                  : { scroll: 120, offset: 90 },
        );
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
        setNode(otherNode);
        await vi.runOnlyPendingTimersAsync();
        // Must resync silently — NOT FLIP from tc-1's 40px.
        expect(el.style.height).toBe("");

        // A genuine branch change on the NEW node (tc-2) afterwards must
        // still be able to FLIP correctly — the reset must not poison the
        // node going forward. It eases from what tc-2 showed (its 500px
        // result view) down to its 120px chunk feed.
        setNode({ ...otherNode, log: { open: true, chunks: [{ kind: "stdout", content: "x", timestamp: 1 }] } });
        expect(el.style.height).toBe("500px"); // frozen "from" synchronously
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("120px"); // eased to the "to" height

        vi.useRealTimers();
    });

    it("a different node swapping in mid-FLIP cancels the outgoing node's transition at once", async () => {
        // ReAgent P1 on #3607: the old per-update re-baseline cancelled any
        // FLIP in flight as a side effect; the swap path must do so itself,
        // or the outgoing tool's pinned height and transition keep running
        // against the incoming tool's content until transitionend (150 ms).
        vi.useFakeTimers();
        stubHeightsByBranch({ scroll: 40, offset: 50 }, { scroll: 120, offset: 80 });
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        setNode(terminalNode); // tc-1: running -> result, a FLIP starts
        expect(el.style.height).toBe("40px");
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("120px"); // mid-transition: no transitionend yet

        setNode({ ...terminalNode, id: "tc-2" }); // slot reuse while the FLIP is in flight
        expect(el.style.height).toBe("");
        expect(el.style.transition).toBe("");
        expect(el.style.overflowY).toBe("");

        vi.useRealTimers();
    });

    it("does not animate when the user prefers reduced motion", async () => {
        reducedMotion = true;
        vi.useFakeTimers();
        stubHeightsByBranch({ scroll: 40, offset: 50 }, { scroll: 120, offset: 80 });
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        setNode(terminalNode);
        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe(""); // branch changed + heights differ, but motion is disabled

        vi.useRealTimers();
    });
});

/**
 * Every stream flush hands each mounted tool log a new `node` object, whether
 * or not anything about that tool changed. The height FLIP used to re-baseline
 * on every one of those updates: `getComputedStyle` on every ancestor (to rule
 * out `content-visibility: hidden`) plus `scrollHeight` and `offsetHeight` —
 * forced style and layout inside the flush, for every tool log in the
 * streaming buffer, every flush. Profiled at ~5 % of main-thread time with
 * three panes streaming (TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md
 * §3.4). Measuring is only needed when the rendered branch changes.
 */
describe("ToolOverlayLog — no layout or style reads unless the branch changes", () => {
    it("node updates within the same branch read no geometry and no computed style", () => {
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        expect(container.querySelector(".agent-tool-overlay-log")).not.toBeNull();

        const reads = { geometry: 0, style: 0 };
        const realGetComputedStyle = window.getComputedStyle;
        vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element, pseudo?: string | null) => {
            reads.style++;
            return realGetComputedStyle(el, pseudo);
        });
        for (const prop of ["scrollHeight", "offsetHeight", "clientHeight"] as const) {
            vi.spyOn(HTMLElement.prototype, prop, "get").mockImplementation(() => (reads.geometry++, 0));
        }

        // Twenty flushes: a fresh node object each time, sometimes with a new
        // chunk, never leaving the "streaming" branch.
        let chunks = streamingNode.log!.chunks;
        for (let i = 0; i < 20; i++) {
            if (i % 2 === 0) chunks = [...chunks, { kind: "stdout", content: `line ${i + 2}`, timestamp: i + 2 }];
            setNode({ ...streamingNode, log: { open: true, chunks } });
        }

        expect(reads.style, "getComputedStyle calls across same-branch updates").toBe(0);
        expect(reads.geometry, "scroll/offset/client height reads across same-branch updates").toBe(0);
    });
});

describe("ToolOverlayLog — magnitude gating uses the rendered height, not scrollHeight (codex P2, PR #2962)", () => {
    it("animates a long-output running->terminal transition even though scrollHeight's own delta is far past the cap", async () => {
        // The real scenario the bug report describes: a raw chunk log near
        // MAX_TOOL_OUTPUT_LINES has a scrollHeight delta easily in the tens
        // of thousands of px against a short terminal result — but the
        // box's RENDERED shrink is bounded by the ancestor panel's
        // max-height to a few hundred px. Gating the magnitude cap on
        // scrollHeight would skip animating exactly this case.
        vi.useFakeTimers();
        stubHeightsByBranch(
            { scroll: 15000, offset: 500 }, // huge raw chunk log, rendered/clamped to the panel's own cap
            { scroll: 200, offset: 220 }, // compact terminal result: rendered delta 280px, well under the cap
        );
        const [node, setNode] = createSignal<ToolNode>(streamingNode);
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        await vi.runOnlyPendingTimersAsync();

        setNode(terminalNode);

        // Synchronous: frozen at the scrollHeight-based FROM value. Checked
        // immediately, not after a timer flush — a naive scrollHeight-gated
        // version sees a ~14800px delta (past MAX_ANIMATED_DELTA_PX) and
        // skips animating entirely, so el.style.height would stay "" here
        // instead.
        expect(el.style.height).toBe("15000px");

        await vi.runOnlyPendingTimersAsync();
        expect(el.style.height).toBe("200px"); // eased to the true (scrollHeight) terminal height

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

    // The test above only proves "bashwrap" text doesn't leak and zero
    // `.agent-tool-log-line` rows render — true both before AND after this
    // fix, since the pre-fix bug rendered an EMPTY ChunkList (also zero
    // rows), not visible text. It doesn't distinguish "correctly blank" from
    // "wrongly blank instead of the Thinking placeholder" — this test does.
    // User report, 2026-09-04: dropBashwrapStartingChunk (above) only fixed
    // what ChunkList renders once it's the active branch; hasChunks() still
    // counted the raw (pre-filter) chunk length, so an all-bashwrap-marker
    // log still routed to ChunkList instead of falling through to
    // ToolOverlayResult's own "still running, nothing to show yet" branch —
    // rendering a visibly empty box between the Working row's "Thinking…"
    // and the first real chunk, instead of a continuous "Thinking…" through
    // both surfaces.
    it("falls through to the Thinking placeholder (not a blank box) when only a bashwrap-starting chunk exists", () => {
        const node: ToolNode = {
            ...streamingNode,
            log: {
                open: true,
                chunks: [
                    { kind: "system", content: "[bashwrap] starting: 42 chars", timestamp: 1 },
                ],
            },
        };
        const { container } = render(() => <ToolOverlayLog node={node} />);
        expect(container.querySelector(".agent-tool-loading")).not.toBeNull();
        expect(container.textContent).toContain("Thinking...");
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
