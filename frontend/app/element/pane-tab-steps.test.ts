// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    ACTIVE_FLOOR_PX,
    ICON_ONLY_BELOW_PX,
    minTabsRoom,
    NO_LABEL_BELOW_PX,
    RESTING_FLOOR_PX,
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
        expect(minTabsRoom(5, RESTING_NO_ICON_FLOOR_PX)).toBe(ACTIVE_FLOOR_PX + 4 * RESTING_NO_ICON_FLOOR_PX);
        expect(RESTING_NO_ICON_FLOOR_PX).toBeGreaterThan(RESTING_FLOOR_PX);
    });
});
