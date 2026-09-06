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
    // truthy, and calls `autoUpdate` against a detached node.
    it("never calls autoUpdate for a hover that ends before the RAF fires", () => {
        vi.useFakeTimers();
        try {
            const row = makeRow();
            const [show, setShow] = createSignal(true);
            render(() => (
                <PeekOverlay show={show()} rowEl={() => row}>
                    <span>peek content</span>
                </PeekOverlay>
            ));
            // Leave BEFORE any timer/RAF has been flushed at all.
            setShow(false);
            vi.advanceTimersByTime(50);
            expect(autoUpdate).not.toHaveBeenCalled();
        } finally {
            vi.useRealTimers();
        }
    });

    it("cleans up autoUpdate's returned disposer on mouseleave (show → false) after the RAF already fired", () => {
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
            expect(disposer).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
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

    it("re-hovering after a full hide→show cycle registers a fresh autoUpdate", () => {
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
