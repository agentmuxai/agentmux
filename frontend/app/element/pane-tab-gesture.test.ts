// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import {
    classifyPillClick,
    markPillDragEnded,
    msSinceLastPillDrag,
    PILL_DRAG_THRESHOLD_PX,
    POST_DRAG_CLICK_GUARD_MS,
    resetPillDragClock,
    wasDrag,
    type PillClickGesture,
} from "./pane-tab-gesture";

const click = (over: Partial<PillClickGesture> = {}): PillClickGesture => ({
    button: 0,
    mod: false,
    shift: false,
    alt: false,
    pressedAt: { x: 10, y: 10 },
    clickedAt: { x: 10, y: 10 },
    dragStartedSincePress: false,
    msSinceLastPillDrag: null,
    ...over,
});

beforeEach(() => resetPillDragClock());

describe("classifyPillClick", () => {
    it("a plain left click expands", () => {
        expect(classifyPillClick(click())).toBe("expand");
    });

    it("a click with no press seen (a synthetic or keyboard click) is plain", () => {
        expect(classifyPillClick(click({ pressedAt: null }))).toBe("expand");
    });

    it("Ctrl/Cmd+click only toggles the pick", () => {
        expect(classifyPillClick(click({ mod: true }))).toBe("toggle-selection");
    });

    it("Shift or Alt+click activates and expands nothing", () => {
        expect(classifyPillClick(click({ shift: true }))).toBe("activate-only");
        expect(classifyPillClick(click({ alt: true }))).toBe("activate-only");
    });

    it("a mod key wins over shift: it is the selection gesture", () => {
        expect(classifyPillClick(click({ mod: true, shift: true }))).toBe("toggle-selection");
    });

    it("another button is not ours: a right-click opens a menu and expands nothing", () => {
        expect(classifyPillClick(click({ button: 2 }))).toBe("ignore");
        expect(classifyPillClick(click({ button: 1 }))).toBe("ignore");
    });

    it("a drag that started on the pill is no click, whatever was held", () => {
        expect(classifyPillClick(click({ dragStartedSincePress: true }))).toBe("ignore");
        expect(classifyPillClick(click({ dragStartedSincePress: true, mod: true }))).toBe("ignore");
    });

    it("the click a drag leaves behind, soon after it ended, is no click", () => {
        expect(classifyPillClick(click({ msSinceLastPillDrag: 0 }))).toBe("ignore");
        expect(classifyPillClick(click({ msSinceLastPillDrag: POST_DRAG_CLICK_GUARD_MS - 1 }))).toBe("ignore");
        expect(classifyPillClick(click({ msSinceLastPillDrag: POST_DRAG_CLICK_GUARD_MS }))).toBe("expand");
    });
});

describe("wasDrag", () => {
    it("allows a slip up to the threshold and not past it", () => {
        const at = (dx: number) => click({ clickedAt: { x: 10 + dx, y: 10 } });
        expect(wasDrag(at(PILL_DRAG_THRESHOLD_PX))).toBe(false);
        expect(wasDrag(at(PILL_DRAG_THRESHOLD_PX + 1))).toBe(true);
        expect(wasDrag(click({ clickedAt: { x: 10, y: 10 + PILL_DRAG_THRESHOLD_PX + 1 } }))).toBe(true);
    });

    it("measures the straight line, so a diagonal slip counts the same", () => {
        expect(wasDrag(click({ clickedAt: { x: 14, y: 14 } }))).toBe(true); // 5.66 px
        expect(wasDrag(click({ clickedAt: { x: 13, y: 13 } }))).toBe(false); // 4.24 px
    });
});

describe("the pill drag clock", () => {
    it("is null until a drag ends, then counts from it", () => {
        expect(msSinceLastPillDrag(5_000)).toBeNull();
        markPillDragEnded(1_000);
        expect(msSinceLastPillDrag(1_200)).toBe(200);
    });
});
