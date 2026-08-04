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
});
