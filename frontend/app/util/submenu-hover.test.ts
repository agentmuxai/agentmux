// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Unit tests for the submenu hover-intent core
// (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10 §5 Phase 1).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createSubmenuHover, type SubmenuHoverController } from "./submenu-hover";

function makeRect(x: number, y: number, w: number, h: number): DOMRect {
    return {
        x,
        y,
        width: w,
        height: h,
        top: y,
        left: x,
        right: x + w,
        bottom: y + h,
        toJSON: () => ({ x, y, width: w, height: h }),
    } as DOMRect;
}

/** Submenu opens to the RIGHT of its trigger, matching this app's default placement. */
const SUBMENU_RECT = makeRect(300, 100, 160, 200); // x:[300,460] y:[100,300]

function moveTo(x: number, y: number) {
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y }));
}

describe("createSubmenuHover", () => {
    let onOpen: ReturnType<typeof vi.fn>;
    let onClose: ReturnType<typeof vi.fn>;
    let controller: SubmenuHoverController;

    beforeEach(() => {
        vi.useFakeTimers();
        onOpen = vi.fn();
        onClose = vi.fn();
        controller = createSubmenuHover({ openDelayMs: 90, closeSafetyTimeoutMs: 300, onOpen, onClose });
    });

    afterEach(() => {
        controller.dispose();
        vi.useRealTimers();
    });

    describe("open delay", () => {
        it("does not open immediately on trigger enter", () => {
            controller.onTriggerEnter();
            expect(onOpen).not.toHaveBeenCalled();
        });

        it("opens after the configured delay of sustained hover", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        });

        it("cancels the pending open if the trigger is left before the delay elapses", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(50);
            controller.onTriggerLeave({ clientX: 0, clientY: 0 });
            vi.advanceTimersByTime(100);
            expect(onOpen).not.toHaveBeenCalled();
            expect(onClose).not.toHaveBeenCalled(); // never opened, so nothing to close
        });

        it("re-entering the same trigger while already open does not re-fire onOpen", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.onTriggerEnter();
            vi.advanceTimersByTime(200);
            expect(onOpen).toHaveBeenCalledOnce();
        });
    });

    describe("safe-triangle close", () => {
        function openSubmenu() {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        }

        it("stays open while the cursor moves diagonally toward the submenu", () => {
            openSubmenu();
            // Leave the trigger row just left of the submenu, vertically centered.
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // Advance through a path that stays inside the triangle toward (300,150)/(300,250).
            moveTo(270, 195);
            vi.advanceTimersByTime(10);
            moveTo(290, 190);
            vi.advanceTimersByTime(10);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("closes promptly when the cursor moves away from the submenu", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // Straight up and away — outside the triangle immediately.
            moveTo(250, 0);
            vi.advanceTimersByTime(10);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("stays open indefinitely once the cursor arrives inside the submenu panel", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            controller.onSubmenuEnter();
            // Well past the safety timeout — should never close while "inside".
            vi.advanceTimersByTime(10_000);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("closes after leaving the submenu panel itself", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            controller.onSubmenuEnter();
            controller.onSubmenuLeave({ clientX: 380, clientY: 200 });
            vi.advanceTimersByTime(300);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("falls back to the safety timeout if the cursor never moves after leaving", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            expect(onClose).not.toHaveBeenCalled();
            vi.advanceTimersByTime(299);
            expect(onClose).not.toHaveBeenCalled();
            vi.advanceTimersByTime(1);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("falls back to a plain delay when no submenu geometry has been registered", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            vi.advanceTimersByTime(300);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("re-entering the trigger mid-close cancels the close", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            vi.advanceTimersByTime(100);
            controller.onTriggerEnter();
            vi.advanceTimersByTime(300);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("reads the submenu rect fresh on each move (survives a mid-hover reposition)", () => {
            let rect = SUBMENU_RECT;
            controller.setSubmenuEl({ getBoundingClientRect: () => rect });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);

            // Submenu got shifted further right by autoUpdate mid-hover.
            rect = makeRect(500, 100, 160, 200);
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // A path toward the OLD rect location would now be outside the triangle
            // for the NEW rect if we were using stale geometry; toward the new
            // location it should stay open.
            moveTo(450, 195);
            vi.advanceTimersByTime(10);
            expect(onClose).not.toHaveBeenCalled();
        });
    });

    describe("close", () => {
        it("closes immediately, bypassing any safe-triangle grace period", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.close();
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("cancels a pending open without ever calling onOpen or onClose", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(50);
            controller.close();
            vi.advanceTimersByTime(100);
            expect(onOpen).not.toHaveBeenCalled();
            expect(onClose).not.toHaveBeenCalled();
        });

        it("is a no-op when already closed", () => {
            controller.close();
            expect(onClose).not.toHaveBeenCalled();
        });

        it("leaves the controller reusable for a later onTriggerEnter", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            controller.close();
            onOpen.mockClear();

            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        });
    });

    describe("dispose", () => {
        it("closes an open submenu immediately, same as close()", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.dispose();
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("removes the mousemove listener and stops pending timers — no further onClose after teardown", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });

            controller.dispose();
            onClose.mockClear();
            moveTo(250, 0);
            vi.advanceTimersByTime(500);
            expect(onClose).not.toHaveBeenCalled();
        });
    });
});
