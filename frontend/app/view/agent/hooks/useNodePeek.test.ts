// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useNodePeek } from "./useNodePeek";
import * as pointerDragState from "@/app/util/pointer-drag-state";

describe("useNodePeek", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
    });

    it("does not peek before the delay elapses", () => {
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(49);
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("peeks once the delay elapses", () => {
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(true);
            dispose();
        });
    });

    it("a leave before the delay elapses cancels the pending peek", () => {
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(30);
            peek.handlePeekLeave();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("a leave after peeking has started closes it immediately", () => {
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(true);
            peek.handlePeekLeave();
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("defaults to the shared 50ms delay when none is passed", () => {
        createRoot((dispose) => {
            const peek = useNodePeek();
            peek.handlePeekEnter();
            vi.advanceTimersByTime(49);
            expect(peek.isPeeking()).toBe(false);
            vi.advanceTimersByTime(1);
            expect(peek.isPeeking()).toBe(true);
            dispose();
        });
    });

    it("does not start peeking on enter while the primary button is held", () => {
        vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(true);
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("still closes an already-showing peek on leave even while the primary button is held (reagentx P1 on PR #3470 — a frozen leave got stuck open forever)", () => {
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(false);
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(true);

            spy.mockReturnValue(true);
            peek.handlePeekLeave();
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("does not start peeking if the button goes down during the pending enter delay", () => {
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(false);
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(30);
            spy.mockReturnValue(true);
            vi.advanceTimersByTime(20);
            expect(peek.isPeeking()).toBe(false);
            dispose();
        });
    });

    it("resumes normal enter/leave behavior once the button is released", () => {
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(true);
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(false);

            spy.mockReturnValue(false);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(true);
            dispose();
        });
    });

    // A menu that closes on a click uncovers the row behind it, and the browser reports that row as entered
    // although the pointer never moved onto it (seen with the window-tab colour menu over a transcript).
    describe("a row that came under a pointer still resting where it pressed", () => {
        const HERE = { clientX: 100, clientY: 20 };
        const row = () => document.createElement("div");
        const move = (el: HTMLElement, clientX: number, clientY: number) =>
            el.dispatchEvent(new PointerEvent("pointermove", { clientX, clientY, bubbles: true }));
        // The pointer rests at HERE until told otherwise, as pointer-drag-state.ts would report it.
        const resting = (isResting = true) =>
            vi.spyOn(pointerDragState, "isRestingAtPress").mockImplementation((e) => isResting && e.clientX === HERE.clientX && e.clientY === HERE.clientY);

        it("does not open the peek", () => {
            resting();
            createRoot((dispose) => {
                const peek = useNodePeek(50);
                peek.setRowEl(row());
                peek.handlePeekEnter(HERE);
                vi.advanceTimersByTime(500);
                expect(peek.isPeeking()).toBe(false);
                dispose();
            });
        });

        it("opens it, after the usual delay, once the pointer moves on the row", () => {
            resting();
            createRoot((dispose) => {
                const peek = useNodePeek(50);
                const el = row();
                peek.setRowEl(el);
                peek.handlePeekEnter(HERE);
                // A hand does not hold the mouse dead still: drift that is still resting opens nothing.
                move(el, HERE.clientX, HERE.clientY);
                vi.advanceTimersByTime(500);
                expect(peek.isPeeking()).toBe(false);
                move(el, 140, 24);
                vi.advanceTimersByTime(49);
                expect(peek.isPeeking()).toBe(false);
                vi.advanceTimersByTime(1);
                expect(peek.isPeeking()).toBe(true);
                dispose();
            });
        });

        it("stays shut when the button that uncovered it is released over it", () => {
            resting();
            const releases: (() => void)[] = [];
            vi.spyOn(pointerDragState, "onPrimaryButtonRelease").mockImplementation((cb) => {
                releases.push(cb);
                return () => {};
            });
            createRoot((dispose) => {
                const peek = useNodePeek(50);
                const el = row();
                vi.spyOn(el, "matches").mockReturnValue(true);
                peek.setRowEl(el);
                peek.handlePeekEnter(HERE);
                for (const cb of releases) cb();
                vi.advanceTimersByTime(500);
                expect(peek.isPeeking()).toBe(false);
                dispose();
            });
        });

        it("forgets the wait when the pointer leaves, and opens normally when it comes back", () => {
            const rest = resting();
            createRoot((dispose) => {
                const peek = useNodePeek(50);
                const el = row();
                peek.setRowEl(el);
                peek.handlePeekEnter(HERE);
                peek.handlePeekLeave();
                // A move over the row after the leave (a late event) opens nothing: it no longer listens.
                move(el, 140, 24);
                vi.advanceTimersByTime(500);
                expect(peek.isPeeking()).toBe(false);
                rest.mockReturnValue(false);
                peek.handlePeekEnter({ clientX: 140, clientY: 24 });
                vi.advanceTimersByTime(50);
                expect(peek.isPeeking()).toBe(true);
                dispose();
            });
        });

        it("opens as before for an enter somewhere else, and for a caller that passes no event", () => {
            resting();
            createRoot((dispose) => {
                const peek = useNodePeek(50);
                peek.setRowEl(row());
                peek.handlePeekEnter({ clientX: 300, clientY: 200 });
                vi.advanceTimersByTime(50);
                expect(peek.isPeeking()).toBe(true);
                peek.handlePeekLeave();
                peek.handlePeekEnter();
                vi.advanceTimersByTime(50);
                expect(peek.isPeeking()).toBe(true);
                dispose();
            });
        });
    });

    it("setRowEl/rowEl round-trip the hovered element", () => {
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            expect(peek.rowEl()).toBeUndefined();
            const el = document.createElement("div");
            peek.setRowEl(el);
            expect(peek.rowEl()).toBe(el);
            dispose();
        });
    });
});
