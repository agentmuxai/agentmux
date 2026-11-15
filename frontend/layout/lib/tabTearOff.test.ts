// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One tab dragged out of a pane that keeps others (tabTearOff.ts): its own place offers the tear-off from the
 * drag's start, stops for good the first time anything else is shown, and the pane then shows the tab has left
 * it until the drag ends, or through a release that tears it off.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LayoutModel } from "./layoutModel";
import {
    TAB_TEAR_OFF_WAIT_MS,
    beginTabTearOff,
    endTabTearOff,
    holdTabTearOff,
    isTabGone,
    tabGoneFrom,
    tabTearOffHeld,
    tabTearOffOffered,
    tabTearOffOfferedIn,
    tabTearOffShown,
} from "./tabTearOff";

const model = {} as LayoutModel;
const other = {} as LayoutModel;

beforeEach(() => {
    vi.useFakeTimers();
    endTabTearOff();
});
afterEach(() => vi.useRealTimers());

describe("a tab dragged out of a pane that keeps others", () => {
    it("offers the tear-off in its own pane from the drag's start, and nowhere else", () => {
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        beginTabTearOff(model, "tab-1", "pane-a");
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(true);
        expect(tabTearOffOffered("tab-1")).toBe(true);
        expect(tabTearOffOfferedIn(model, "pane-b")).toBe(false);
        expect(tabTearOffOfferedIn(other, "pane-a")).toBe(false);
        expect(tabTearOffOffered("tab-2")).toBe(false);
        // The tab is still in its place: the pane does not show it gone.
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();
        expect(isTabGone("tab-1", "pane-a")).toBe(false);
    });

    it("has left the pane once anything else is shown, and the offer does not come back", () => {
        beginTabTearOff(model, "tab-1", "pane-a");
        tabTearOffShown();
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        expect(tabTearOffOffered("tab-1")).toBe(false);
        expect(tabGoneFrom(model, "pane-a")).toBe("tab-1");
        expect(isTabGone("tab-1", "pane-a")).toBe(true);
        // Only that tab, of that pane, in that layout.
        expect(isTabGone("tab-2", "pane-a")).toBe(false);
        expect(isTabGone("tab-1", "pane-b")).toBe(false);
        expect(tabGoneFrom(model, "pane-b")).toBeUndefined();
        expect(tabGoneFrom(other, "pane-a")).toBeUndefined();
        // Shown again (back over its own pane, out of the window a second time): nothing changes.
        tabTearOffShown();
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        expect(tabGoneFrom(model, "pane-a")).toBe("tab-1");
    });

    it("is over when the drag ends with no tear-off, and the next drag starts with the offer", () => {
        beginTabTearOff(model, "tab-1", "pane-a");
        tabTearOffShown();
        endTabTearOff();
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();
        expect(isTabGone("tab-1", "pane-a")).toBe(false);
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        beginTabTearOff(model, "tab-1", "pane-a");
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(true);
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();
    });

    it("stays gone through a release that tears it off, until the tear-off is done", () => {
        // Released over its own place while that offered the tear-off: the red goes, the tab does not come back.
        beginTabTearOff(model, "tab-1", "pane-a");
        holdTabTearOff();
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        expect(tabGoneFrom(model, "pane-a")).toBe("tab-1");
        expect(tabTearOffHeld()).toBe(true);
        vi.advanceTimersByTime(TAB_TEAR_OFF_WAIT_MS - 1);
        expect(tabGoneFrom(model, "pane-a")).toBe("tab-1");
        endTabTearOff();
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();
        expect(tabTearOffHeld()).toBe(false);
    });

    it("comes back when a released tear-off never happens, and a new drag is not cut short by the old wait", () => {
        beginTabTearOff(model, "tab-1", "pane-a");
        tabTearOffShown();
        holdTabTearOff();
        vi.advanceTimersByTime(TAB_TEAR_OFF_WAIT_MS);
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();

        beginTabTearOff(model, "tab-1", "pane-a");
        holdTabTearOff();
        vi.advanceTimersByTime(TAB_TEAR_OFF_WAIT_MS / 2);
        // The next drag starts before the old wait runs out.
        beginTabTearOff(model, "tab-2", "pane-a");
        vi.advanceTimersByTime(TAB_TEAR_OFF_WAIT_MS);
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(true);
        expect(tabTearOffOffered("tab-2")).toBe(true);
    });

    it("takes nothing down and holds nothing when no tab is dragged", () => {
        tabTearOffShown();
        holdTabTearOff();
        expect(tabTearOffOfferedIn(model, "pane-a")).toBe(false);
        expect(tabGoneFrom(model, "pane-a")).toBeUndefined();
    });
});
