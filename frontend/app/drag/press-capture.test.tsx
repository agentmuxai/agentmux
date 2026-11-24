// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { armPressCapture, disarmPressCapture, PRESS_DRAG_PX, PRESS_HOLD_MS } from "./press-capture";

const at = { x: 100, y: 50 };
const fire = (type: string, x = at.x, y = at.y) => document.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, bubbles: true }));

describe("a press on a pane's header and the picture of the pane", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        disarmPressCapture();
        vi.useRealTimers();
    });

    it("takes no picture for a click: released before it moves or is held", () => {
        const capture = vi.fn();
        armPressCapture(at, capture);
        fire("pointermove", at.x + 1, at.y);
        vi.advanceTimersByTime(PRESS_HOLD_MS - 10);
        fire("pointerup");
        vi.advanceTimersByTime(PRESS_HOLD_MS * 4);
        fire("pointermove", at.x + 40, at.y);
        expect(capture).not.toHaveBeenCalled();
    });

    it("takes it once the pointer has moved a few pixels, and only once", () => {
        const capture = vi.fn();
        armPressCapture(at, capture);
        fire("pointermove", at.x + PRESS_DRAG_PX - 1, at.y);
        expect(capture).not.toHaveBeenCalled();
        fire("pointermove", at.x + PRESS_DRAG_PX, at.y);
        expect(capture).toHaveBeenCalledTimes(1);
        fire("pointermove", at.x + 30, at.y);
        fire("dragstart");
        vi.advanceTimersByTime(PRESS_HOLD_MS * 2);
        expect(capture).toHaveBeenCalledTimes(1);
    });

    it("takes it when the library's drag starts or the pointer is cancelled by a native drag, before any move was seen", () => {
        const onDragStart = vi.fn();
        armPressCapture(at, onDragStart);
        fire("dragstart");
        expect(onDragStart).toHaveBeenCalledTimes(1);

        const onCancel = vi.fn();
        armPressCapture(at, onCancel);
        fire("pointercancel");
        expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it("takes it when the pointer is held down without moving", () => {
        const capture = vi.fn();
        armPressCapture(at, capture);
        vi.advanceTimersByTime(PRESS_HOLD_MS - 1);
        expect(capture).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(capture).toHaveBeenCalledTimes(1);
    });

    it("watches one press: a new press forgets the one before it, and disarming forgets the press", () => {
        const first = vi.fn();
        const second = vi.fn();
        armPressCapture(at, first);
        armPressCapture({ x: 300, y: 300 }, second);
        fire("pointermove", at.x + 20, at.y);
        expect(first).not.toHaveBeenCalled();
        expect(second).toHaveBeenCalledTimes(1);

        const third = vi.fn();
        armPressCapture(at, third);
        disarmPressCapture();
        vi.advanceTimersByTime(PRESS_HOLD_MS * 2);
        fire("dragstart");
        expect(third).not.toHaveBeenCalled();
    });
});
