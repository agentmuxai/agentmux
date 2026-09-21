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

    it("freezes an already-showing peek through a leave while the primary button is held", () => {
        const spy = vi.spyOn(pointerDragState, "isPrimaryButtonDown").mockReturnValue(false);
        createRoot((dispose) => {
            const peek = useNodePeek(50);
            peek.handlePeekEnter();
            vi.advanceTimersByTime(50);
            expect(peek.isPeeking()).toBe(true);

            spy.mockReturnValue(true);
            peek.handlePeekLeave();
            expect(peek.isPeeking()).toBe(true);
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
