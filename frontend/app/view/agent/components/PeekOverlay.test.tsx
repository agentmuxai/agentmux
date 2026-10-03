// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekOverlay — Portal-rendered hover-to-peek panel.
 *
 * reagent P1 on PR #2392: a rapid hover→leave (very reachable given the
 * 150ms enter-delay every caller uses before setting `show`, then a
 * mouseleave arriving within the same animation frame) used to let a
 * stale `requestAnimationFrame` callback fire AFTER this component's
 * `<Show>` branch had already unmounted the floating div — `floatingEl`
 * was never reset and the RAF was never cancelled, so `autoUpdate()` ran
 * anyway against a detached node, registering scroll/resize listeners
 * nothing would ever clean up. These tests drive that exact race with
 * fake timers and a mocked `autoUpdate`.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { autoUpdate } from "@floating-ui/dom";
import { PeekOverlay } from "./PeekOverlay";
import { useNodePeek, type NodePeek } from "../hooks/useNodePeek";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
}));

afterEach(() => {
    cleanup();
    vi.mocked(autoUpdate).mockClear();
});

function makeRow(): HTMLElement {
    const row = document.createElement("div");
    document.body.appendChild(row);
    return row;
}

describe("PeekOverlay", () => {
    it("renders children when show is true", () => {
        // Portal-rendered at document.body — `screen` queries the whole
        // document, unlike `render()`'s own container-scoped queries.
        const row = makeRow();
        render(() => (
            <PeekOverlay show={true} rowEl={() => row}>
                <span>peek content</span>
            </PeekOverlay>
        ));
        expect(screen.getByText("peek content")).toBeInTheDocument();
    });

    it("renders nothing when show is false", () => {
        const row = makeRow();
        render(() => (
            <PeekOverlay show={false} rowEl={() => row}>
                <span>peek content</span>
            </PeekOverlay>
        ));
        expect(screen.queryByText("peek content")).toBeNull();
    });

    it("registers autoUpdate once the mount's RAF fires while still shown", () => {
        vi.useFakeTimers();
        try {
            const row = makeRow();
            render(() => (
                <PeekOverlay show={true} rowEl={() => row}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            vi.advanceTimersByTime(50); // flush the RAF
            expect(autoUpdate).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    // The bug this regression-tests: show flips true→false BEFORE the
    // mount's RAF has a chance to run. Without the fix, the RAF fires
    // anyway (nothing cancelled it), finds the stale `floatingEl` still
    // truthy, and calls `autoUpdate` against a detached node. The panel now
    // lingers HOVER_BRIDGE_MS after `show` goes false (hover bridge), so the RAF
    // may legitimately register against the still-mounted panel; what must hold
    // is that nothing is left registered once it has closed.
    it("leaves no autoUpdate registered for a hover that ends before the RAF fires", () => {
        vi.useFakeTimers();
        try {
            const row = makeRow();
            const disposer = vi.fn();
            vi.mocked(autoUpdate).mockReturnValue(disposer);
            const [show, setShow] = createSignal(true);
            render(() => (
                <PeekOverlay show={show()} rowEl={() => row}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            // Leave BEFORE any timer/RAF has been flushed at all.
            setShow(false);
            vi.advanceTimersByTime(500); // RAF, then the linger, both elapse
            expect(document.querySelector(".agent-node-peek-overlay")).toBeNull();
            expect(disposer).toHaveBeenCalledTimes(vi.mocked(autoUpdate).mock.calls.length);
        } finally {
            vi.mocked(autoUpdate).mockReset();
            vi.mocked(autoUpdate).mockImplementation(() => vi.fn());
            vi.useRealTimers();
        }
    });

    it("cleans up autoUpdate's returned disposer on mouseleave (show → false, after the linger) after the RAF already fired", () => {
        vi.useFakeTimers();
        try {
            const row = makeRow();
            const disposer = vi.fn();
            vi.mocked(autoUpdate).mockReturnValueOnce(disposer);
            const [show, setShow] = createSignal(true);
            render(() => (
                <PeekOverlay show={show()} rowEl={() => row}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            vi.advanceTimersByTime(50); // RAF fires, autoUpdate registers
            expect(autoUpdate).toHaveBeenCalledTimes(1);
            expect(disposer).not.toHaveBeenCalled();
            setShow(false);
            vi.advanceTimersByTime(500); // the hover-bridge linger elapses, then it unmounts
            expect(disposer).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    // Placement and the hover bridge. Row 100-500 wide and 200-230 tall, transcript
    // 100-400, window 1400x1000, pointer inside the row. A panel that fits follows
    // the pointer and closes the instant the pointer leaves the row. One too tall
    // to fit is meant to be ENTERED: pinned (holds still), flush with the row,
    // lingering after the row's mouseleave (longer while the pointer approaches),
    // and other rows' peeks wait for it. Before the pin, every move toward a tall
    // panel moved it away and cut its height, so it could never be reached.
    describe("placement and hover bridge", () => {
        const panel = () => document.querySelector(".agent-node-peek-overlay") as HTMLElement | null;
        const rectOf = (el: Element, r: Partial<DOMRect>) =>
            vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
                top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => {}, ...r,
            } as DOMRect);
        const topOf = () => parseFloat(panel()!.style.top);

        function setup(opts: { align?: "end" | "stretch"; panelHeight: number; panelWidth?: number; mouseY?: number }) {
            const container = document.createElement("div");
            container.style.overflowY = "auto";
            document.body.appendChild(container);
            const row = document.createElement("div");
            container.appendChild(row);
            rectOf(container, { top: 100, bottom: 400 });
            rectOf(row, { top: 200, bottom: 230, left: 100, right: 500, width: 400 });
            Object.defineProperty(window, "innerWidth", { value: 1400, configurable: true });
            Object.defineProperty(window, "innerHeight", { value: 1000, configurable: true });
            const [show, setShow] = createSignal(true);
            render(() => (
                <PeekOverlay show={show()} rowEl={() => row} align={opts.align}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            vi.advanceTimersByTime(50);
            // Its measured size, and (for the approach tests) where it sits.
            rectOf(panel()!, {
                height: opts.panelHeight, width: opts.panelWidth ?? 300, top: 230, bottom: 830, left: 200, right: 500,
            });
            const moveTo = (y: number) => {
                row.dispatchEvent(new MouseEvent("mousemove", { clientY: y, bubbles: true }));
                vi.advanceTimersByTime(50);
            };
            moveTo(opts.mouseY ?? 215);
            return { row, setShow, moveTo };
        }
        /** The pointer moving over the page (not the row), as during the crossing. */
        const pointerAt = (x: number, y: number) =>
            document.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y }));
        /** A row elsewhere using the real hover hook, to watch whether its peek opens. */
        function otherRow(rowEl?: HTMLElement): NodePeek {
            let peek!: NodePeek;
            const el = rowEl ?? document.createElement("div");
            function Probe() {
                peek = useNodePeek();
                peek.setRowEl(el);
                return null;
            }
            render(() => <Probe />);
            return peek;
        }

        it("a panel that fits follows the pointer, as before", () => {
            vi.useFakeTimers();
            try {
                const { moveTo } = setup({ panelHeight: 40 });
                expect(topOf()).toBe(215 + 12);
                moveTo(222);
                expect(topOf()).toBe(222 + 12);
            } finally {
                vi.useRealTimers();
            }
        });

        it("a panel too tall to fit is pinned: it holds still, and keeps its height, while the pointer moves toward it", () => {
            vi.useFakeTimers();
            try {
                const { moveTo } = setup({ panelHeight: 800 });
                const top = topOf();
                const maxHeight = panel()!.style.maxHeight;
                expect(top).toBe(215 + 12);
                moveTo(222);
                moveTo(228);
                expect(topOf()).toBe(top);
                expect(panel()!.style.maxHeight).toBe(maxHeight);
            } finally {
                vi.useRealTimers();
            }
        });

        it("a pinned panel's near edge is flush with the row, so reaching it crosses no other row", () => {
            vi.useFakeTimers();
            try {
                // Pointer near the row's bottom: pointer + 12 would leave a strip of the next row.
                setup({ panelHeight: 800, mouseY: 228 });
                expect(topOf()).toBe(230); // the row's bottom edge, not 240
                expect(panel()!.hasAttribute("data-pane-overlay")).toBe(true);
            } finally {
                vi.useRealTimers();
            }
        });

        it("a pinned panel still moves with its row when the transcript scrolls", () => {
            vi.useFakeTimers();
            try {
                const { row } = setup({ panelHeight: 800 });
                expect(topOf()).toBe(227);
                const update = vi.mocked(autoUpdate).mock.calls.at(-1)![2] as () => void;
                rectOf(row, { top: 150, bottom: 180, left: 100, right: 500, width: 400 });
                update();
                expect(topOf()).toBe(150 + 15 + 12); // same offset into the row as when it was pinned
            } finally {
                vi.useRealTimers();
            }
        });

        it("a panel wider than the row pins to the row's left edge and extends right", () => {
            vi.useFakeTimers();
            try {
                setup({ panelHeight: 40, panelWidth: 900 });
                expect(panel()!.style.left).toBe("100px");
                expect(panel()!.style.transform).toBe("");
                expect(parseFloat(panel()!.style.maxWidth)).toBe(1400 - 8 - 100);
            } finally {
                vi.useRealTimers();
            }
        });

        it("a panel that fits stays inside the pane, untagged, and closes at once", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 40 });
                expect(panel()!.hasAttribute("data-pane-overlay")).toBe(false);
                setShow(false);
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("an enterable panel lingers after the row's mouseleave, then closes", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                expect(panel()).not.toBeNull();
                vi.advanceTimersByTime(200);
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("the linger is re-armed while the pointer keeps closing in on the panel", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                // Panel spans y 230-830; the pointer approaches from above, slowly.
                for (const y of [150, 170, 190, 210]) {
                    vi.advanceTimersByTime(100);
                    pointerAt(300, y);
                }
                vi.advanceTimersByTime(100); // 500 ms in, well past the plain 150 ms
                expect(panel()).not.toBeNull();
                vi.advanceTimersByTime(200); // stopped moving: it closes
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("a pointer moving away does not keep it open", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                vi.advanceTimersByTime(50);
                pointerAt(300, 100); // first sighting (re-arms once: 50 + 150 = 200)
                vi.advanceTimersByTime(50);
                pointerAt(300, 60); // farther: no re-arm
                vi.advanceTimersByTime(110);
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("an approach cannot keep it open past the cap", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                for (let i = 1; i <= 12; i++) {
                    vi.advanceTimersByTime(100);
                    pointerAt(300, i * 5); // always a little closer
                }
                expect(panel()).toBeNull(); // 1200 ms of approaching, cap is 1000
            } finally {
                vi.useRealTimers();
            }
        });

        it("stays open while the pointer is on the panel, and closes after it leaves", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                panel()!.dispatchEvent(new MouseEvent("mouseenter"));
                vi.advanceTimersByTime(2000);
                expect(panel()).not.toBeNull();
                panel()!.dispatchEvent(new MouseEvent("mouseleave"));
                vi.advanceTimersByTime(200);
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("a re-enter of the row (show to true) cancels the linger", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                setShow(false);
                vi.advanceTimersByTime(50);
                setShow(true);
                vi.advanceTimersByTime(1000);
                expect(panel()).not.toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        it("while the pointer crosses to an enterable panel, a row it passes over waits; it opens if the pointer never arrives", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                const passedOver = otherRow();
                setShow(false); // left this row, heading for the panel
                passedOver.handlePeekEnter();
                vi.advanceTimersByTime(60); // its enter delay (50 ms) has elapsed
                expect(passedOver.isPeeking()).toBe(false); // waiting, not on top of the panel
                vi.advanceTimersByTime(150); // the panel's grace ran out
                expect(panel()).toBeNull();
                expect(passedOver.isPeeking()).toBe(true); // still hovered, so it opens now
            } finally {
                vi.useRealTimers();
            }
        });

        it("...and never opens if the pointer reached the panel (it left that row on the way)", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ panelHeight: 5000 });
                const passedOver = otherRow();
                setShow(false);
                passedOver.handlePeekEnter();
                vi.advanceTimersByTime(60);
                passedOver.handlePeekLeave();
                panel()!.dispatchEvent(new MouseEvent("mouseenter"));
                vi.advanceTimersByTime(1000);
                expect(panel()).not.toBeNull();
                expect(passedOver.isPeeking()).toBe(false);
            } finally {
                vi.useRealTimers();
            }
        });

        it("the panel's own row is never held off by its bridge", () => {
            vi.useFakeTimers();
            try {
                const { row, setShow } = setup({ panelHeight: 5000 });
                const ownRow = otherRow(row);
                setShow(false);
                ownRow.handlePeekEnter();
                vi.advanceTimersByTime(60);
                expect(ownRow.isPeeking()).toBe(true);
            } finally {
                vi.useRealTimers();
            }
        });

        it("does not bridge the stretch variant, which sits flush over its row", () => {
            vi.useFakeTimers();
            try {
                const { setShow } = setup({ align: "stretch", panelHeight: 5000 });
                setShow(false);
                expect(panel()).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });
    });

    // Mouse-Y tracking (align="end" default) — SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md.
    // These exercise the exact invariant CURSOR_GAP_PX exists to guarantee:
    // the cursor's Y must never fall inside the rendered overlay's own
    // [top, top + height] bounds, or the row's mouseleave/mouseenter fire
    // back-to-back and the panel flickers (reagent P1, 2nd/3rd/4th rounds
    // on PR #2949).
    describe("mouse-Y tracking", () => {
        function setRect(el: Element, rect: Partial<DOMRect>) {
            vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
                top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => {},
                ...rect,
            } as DOMRect);
        }

        function makeScrollableRow(containerRect: Partial<DOMRect>, rowRect: Partial<DOMRect>) {
            const container = document.createElement("div");
            container.style.overflowY = "auto";
            document.body.appendChild(container);
            const row = document.createElement("div");
            container.appendChild(row);
            setRect(container, containerRect);
            setRect(row, rowRect);
            return row;
        }

        it("positions the panel below the cursor, with room to spare", () => {
            vi.useFakeTimers();
            try {
                const row = makeScrollableRow(
                    { top: 0, bottom: 1000, right: 300 },
                    { top: 100, bottom: 500, right: 300, width: 300 },
                );
                render(() => (
                    <PeekOverlay show={true} rowEl={() => row}>
                        <span>peek content</span>
                    </PeekOverlay>
                ));
                vi.advanceTimersByTime(50);
                const overlay = document.querySelector(".agent-node-peek-overlay") as HTMLElement;
                setRect(overlay, { height: 40 });

                const mouseY = 200;
                row.dispatchEvent(new MouseEvent("mousemove", { clientY: mouseY, bubbles: true }));
                vi.advanceTimersByTime(50);

                const top = parseFloat(overlay.style.top);
                expect(top).toBeGreaterThan(mouseY); // below the cursor, not at/above it
            } finally {
                vi.useRealTimers();
            }
        });

        // reagent P1 on PR #2949 (4th round): clamping `top` to fit the
        // overlay within the container's bottom edge used to override the
        // below-the-cursor placement whenever the cursor was within
        // `overlayHeight + BOTTOM_MARGIN_PX` of that edge, landing `top`
        // at or below the cursor's own Y — reintroducing the cursor-inside-
        // the-overlay flicker the gap offset exists to prevent.
        it("flips the panel ABOVE the cursor instead of clamping into it, near the scroll container's bottom edge", () => {
            vi.useFakeTimers();
            try {
                const row = makeScrollableRow(
                    { top: 0, bottom: 220, right: 300 },
                    { top: 100, bottom: 220, right: 300, width: 300 },
                );
                render(() => (
                    <PeekOverlay show={true} rowEl={() => row}>
                        <span>peek content</span>
                    </PeekOverlay>
                ));
                vi.advanceTimersByTime(50);
                const overlay = document.querySelector(".agent-node-peek-overlay") as HTMLElement;
                const overlayHeight = 40;
                setRect(overlay, { height: overlayHeight });

                // Close enough to container.bottom (220) that below-with-gap
                // (mouseY + 12) + overlayHeight (40) would exceed it.
                const mouseY = 210;
                row.dispatchEvent(new MouseEvent("mousemove", { clientY: mouseY, bubbles: true }));
                vi.advanceTimersByTime(50);

                const top = parseFloat(overlay.style.top);
                // The invariant: cursor must land strictly outside [top, top+height].
                expect(top + overlayHeight <= mouseY || top > mouseY).toBe(true);
                // Specifically: flips above (bottom edge of the panel sits
                // above the cursor), not clamped down onto/past it.
                expect(top + overlayHeight).toBeLessThanOrEqual(mouseY);
            } finally {
                vi.useRealTimers();
            }
        });
    });

    it("re-hovering after a full hide (past the linger) registers a fresh autoUpdate", () => {
        vi.useFakeTimers();
        try {
            const row = makeRow();
            const [show, setShow] = createSignal(true);
            render(() => (
                <PeekOverlay show={show()} rowEl={() => row}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            vi.advanceTimersByTime(50);
            expect(autoUpdate).toHaveBeenCalledTimes(1);
            setShow(false);
            vi.advanceTimersByTime(500); // full close: the linger elapses, the panel unmounts
            setShow(true);
            vi.advanceTimersByTime(50);
            expect(autoUpdate).toHaveBeenCalledTimes(2);
        } finally {
            vi.useRealTimers();
        }
    });

    // The panel is Portal-rendered to document.body, which escapes the agent
    // pane's `zoom` — so it used to paint at 100% while the pane around it
    // scaled. It now reads `--agent-pane-zoom` off the anchor row (the var
    // inherits down from the pane root) and applies it to itself.
    describe("agent pane zoom", () => {
        function setRect(el: Element, rect: Partial<DOMRect>) {
            vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
                top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => {},
                ...rect,
            } as DOMRect);
        }

        /** A row inside a scroll container inside a zoomed pane root. */
        function makeZoomedRow(paneZoom: string | null) {
            const paneRoot = document.createElement("div");
            if (paneZoom != null) paneRoot.style.setProperty("--agent-pane-zoom", paneZoom);
            document.body.appendChild(paneRoot);
            const container = document.createElement("div");
            container.style.overflowY = "auto";
            paneRoot.appendChild(container);
            const row = document.createElement("div");
            container.appendChild(row);
            setRect(container, { top: 0, bottom: 1000, right: 400 });
            setRect(row, { top: 100, bottom: 300, left: 100, right: 400, width: 300 });
            return row;
        }

        function renderPeek(row: HTMLElement, align?: "end" | "stretch") {
            render(() => (
                <PeekOverlay show={true} rowEl={() => row} align={align}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            vi.advanceTimersByTime(50);
            return document.querySelector(".agent-node-peek-overlay") as HTMLElement;
        }

        it("applies the pane's zoom factor to the portaled panel", () => {
            vi.useFakeTimers();
            try {
                const overlay = renderPeek(makeZoomedRow("1.5"));
                expect(overlay.style.zoom).toBe("1.5");
            } finally {
                vi.useRealTimers();
            }
        });

        // The non-obvious half: CSS `zoom` also multiplies the element's own
        // inset lengths, and every input here is an already-post-zoom
        // getBoundingClientRect() value — so each must be pre-divided to land
        // where it did before. Without this the panel would fly off-screen at
        // high zoom instead of merely being the wrong size.
        it("pre-divides its viewport-px geometry so the panel still lands on the row's edge", () => {
            vi.useFakeTimers();
            try {
                const overlay = renderPeek(makeZoomedRow("2"));
                // row.right is 400 real px; at zoom 2 that must be written as 200.
                expect(parseFloat(overlay.style.left)).toBeCloseTo(200, 5);
                // max-width tracks the row's 300px width → 150 at zoom 2.
                expect(parseFloat(overlay.style.maxWidth)).toBeCloseTo(150, 5);
            } finally {
                vi.useRealTimers();
            }
        });

        it("de-scales the stretch variant's width and left too", () => {
            vi.useFakeTimers();
            try {
                const overlay = renderPeek(makeZoomedRow("2"), "stretch");
                expect(overlay.style.zoom).toBe("2");
                expect(parseFloat(overlay.style.left)).toBeCloseTo(50, 5);   // 100 / 2
                expect(parseFloat(overlay.style.top)).toBeCloseTo(50, 5);    // 100 / 2
                expect(parseFloat(overlay.style.width)).toBeCloseTo(150, 5); // 300 / 2
            } finally {
                vi.useRealTimers();
            }
        });

        // The overwhelmingly common case must stay byte-identical to the
        // pre-fix behavior — no `zoom` property emitted, no division.
        it("emits no zoom and leaves geometry untouched at 100%", () => {
            vi.useFakeTimers();
            try {
                const overlay = renderPeek(makeZoomedRow("1"));
                expect(overlay.style.zoom).toBe("");
                expect(parseFloat(overlay.style.left)).toBeCloseTo(400, 5);
                expect(parseFloat(overlay.style.maxWidth)).toBeCloseTo(300, 5);
            } finally {
                vi.useRealTimers();
            }
        });

        // A peek rendered outside any agent pane (or in a harness with no
        // computed custom properties) must not break.
        it("falls back to 1 when the pane zoom variable is absent", () => {
            vi.useFakeTimers();
            try {
                const overlay = renderPeek(makeZoomedRow(null));
                expect(overlay.style.zoom).toBe("");
                expect(parseFloat(overlay.style.left)).toBeCloseTo(400, 5);
            } finally {
                vi.useRealTimers();
            }
        });

        it("ignores a malformed or non-positive zoom variable", () => {
            vi.useFakeTimers();
            try {
                expect(renderPeek(makeZoomedRow("not-a-number")).style.zoom).toBe("");
                cleanup();
                expect(renderPeek(makeZoomedRow("0")).style.zoom).toBe("");
            } finally {
                vi.useRealTimers();
            }
        });
    });
});
