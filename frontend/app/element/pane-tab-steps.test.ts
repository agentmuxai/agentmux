// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    ACTIVE_FLOOR_PX,
    ACTIVE_LABEL_MAX_PX,
    ACTIVE_LABEL_MIN_PX,
    activeFloor,
    activeWant,
    ICON_ONLY_BELOW_PX,
    minTabsRoom,
    othersShare,
    NO_LABEL_BELOW_PX,
    RESTING_FLOOR_PX,
    ACTIVE_NO_ICON_FLOOR_PX,
    RESTING_NO_ICON_FLOOR_PX,
    STEP_HYSTERESIS_PX,
    stepFor,
} from "./pane-tab-steps";

describe("stepFor", () => {
    it("is full whenever the tabs fit, whatever the share", () => {
        expect(stepFor(10, true, "icon")).toBe("full");
        expect(stepFor(Infinity, false, "full")).toBe("full");
    });

    it("drops the labels of tabs that are not selected under the first threshold, then to icons under the second", () => {
        expect(stepFor(NO_LABEL_BELOW_PX, false, "full")).toBe("full");
        expect(stepFor(NO_LABEL_BELOW_PX - 1, false, "full")).toBe("icon-close");
        expect(stepFor(ICON_ONLY_BELOW_PX, false, "full")).toBe("icon-close");
        expect(stepFor(ICON_ONLY_BELOW_PX - 1, false, "full")).toBe("icon");
        expect(stepFor(2, false, "icon-close")).toBe("icon");
    });

    it("goes back to a roomier step only with margin past the threshold, so a strip on a boundary does not flap", () => {
        expect(stepFor(NO_LABEL_BELOW_PX, false, "icon-close")).toBe("icon-close");
        expect(stepFor(NO_LABEL_BELOW_PX + STEP_HYSTERESIS_PX - 1, false, "icon-close")).toBe("icon-close");
        expect(stepFor(NO_LABEL_BELOW_PX + STEP_HYSTERESIS_PX, false, "icon-close")).toBe("full");
        expect(stepFor(ICON_ONLY_BELOW_PX, false, "icon")).toBe("icon");
        expect(stepFor(ICON_ONLY_BELOW_PX + STEP_HYSTERESIS_PX, false, "icon")).toBe("icon-close");
    });

    it("can go from icons straight to full when the room is large", () => {
        expect(stepFor(200, false, "icon")).toBe("full");
    });

    it("never goes to a tighter step only because of the margin", () => {
        expect(stepFor(NO_LABEL_BELOW_PX - 1, false, "icon-close")).toBe("icon-close");
        expect(stepFor(ICON_ONLY_BELOW_PX - 1, false, "icon")).toBe("icon");
    });
});

describe("minTabsRoom", () => {
    it("is the selected tab's floor plus the others'", () => {
        expect(minTabsRoom(0)).toBe(0);
        expect(minTabsRoom(1)).toBe(ACTIVE_FLOOR_PX);
        expect(minTabsRoom(5)).toBe(ACTIVE_FLOOR_PX + 4 * RESTING_FLOOR_PX);
    });

    it("takes the wider floor of tabs that have no icon", () => {
        expect(minTabsRoom(5, RESTING_NO_ICON_FLOOR_PX, ACTIVE_NO_ICON_FLOOR_PX)).toBe(
            ACTIVE_NO_ICON_FLOOR_PX + 4 * RESTING_NO_ICON_FLOOR_PX
        );
        expect(RESTING_NO_ICON_FLOOR_PX).toBeGreaterThan(RESTING_FLOOR_PX);
        // The selected one keeps its close button on top of the letters the others keep.
        expect(ACTIVE_NO_ICON_FLOOR_PX).toBeGreaterThan(RESTING_NO_ICON_FLOOR_PX);
        expect(ACTIVE_NO_ICON_FLOOR_PX).toBeGreaterThan(ACTIVE_FLOOR_PX);
    });
});

describe("the selected tab has its room first", () => {
    it("keeps its icon, buttons and close button whole and the start of its title: never less than the icon and close", () => {
        // An agent's tab: icon, six buttons, close, padding.
        expect(activeFloor(190, 400)).toBe(190 + ACTIVE_LABEL_MIN_PX);
        // A short title is kept whole, not padded out.
        expect(activeFloor(190, 20)).toBe(190 + 20);
        // An editor file: nothing but its icon and close button.
        expect(activeFloor(30, 400)).toBe(Math.max(ACTIVE_FLOOR_PX, 30 + ACTIVE_LABEL_MIN_PX));
        expect(activeFloor(10, 0)).toBe(ACTIVE_FLOOR_PX);
    });

    it("asks for its whole title up to the title's cap when nothing is squeezed", () => {
        expect(activeWant(190, 400)).toBe(190 + ACTIVE_LABEL_MAX_PX);
        expect(activeWant(190, 60)).toBe(250);
    });

    it("leaves the other tabs what the room has left of it, shared; with one tab, all of it", () => {
        expect(othersShare(600, 5, 300)).toBe(75);
        expect(othersShare(600, 1, 300)).toBe(600);
        expect(othersShare(200, 5, 300)).toBe(0);
    });

    it("steps the other tabs down by what they are left with, so a selected tab with many buttons turns them to icons sooner", () => {
        // Five tabs in 440 px: an equal share is 88 (labels go), but with an agent tab of 300 the others have 35 (icons).
        expect(stepFor(440 / 5, false, "full")).toBe("icon-close");
        expect(stepFor(othersShare(440, 5, activeWant(190, 400)), false, "full")).toBe("icon");
        // A selected editor file with no buttons asks less of the room.
        expect(stepFor(othersShare(440, 5, activeWant(30, 80)), false, "full")).toBe("icon-close");
    });

    it("counts the selected tab's floor in the least room the strip may shrink to, before its header scrolls", () => {
        expect(minTabsRoom(5, RESTING_FLOOR_PX, 246)).toBe(246 + 4 * RESTING_FLOOR_PX);
        expect(minTabsRoom(5, RESTING_FLOOR_PX, 10)).toBe(ACTIVE_FLOOR_PX + 4 * RESTING_FLOOR_PX);
    });
});
