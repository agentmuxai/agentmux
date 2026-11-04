// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setTabGrabOffset, tabAnchorOffset } from "./tab-grab-offset";

function strip(firstTabLeft: number, firstTabTop: number): HTMLElement {
    const el = document.createElement("div");
    const tab = document.createElement("div");
    tab.className = "tab-drop-wrapper";
    tab.getBoundingClientRect = () => ({ left: firstTabLeft, top: firstTabTop }) as DOMRect;
    el.appendChild(tab);
    return el;
}

describe("tabAnchorOffset", () => {
    const sizes = { outerWidth: window.outerWidth, outerHeight: window.outerHeight };

    beforeEach(() => {
        Object.assign(window, { outerWidth: 1216, outerHeight: 839, innerWidth: 1200, innerHeight: 800 });
    });
    afterEach(() => {
        Object.assign(window, sizes);
        setTabGrabOffset(null);
    });

    it("is the grab in the tab, the first tab in the window and the window's chrome", () => {
        setTabGrabOffset({ x: 40, y: 12 });
        // Chrome: 8 px a side, and the rest of the height less one side border below the title bar: 39 - 8 = 31.
        expect(tabAnchorOffset(strip(30, 6))).toEqual({ x: 40 + 30 + 8, y: 12 + 6 + 31 });
    });

    it("is nothing before a drag has set the grab offset, or without a tab in the strip", () => {
        expect(tabAnchorOffset(strip(30, 6))).toBeNull();
        setTabGrabOffset({ x: 40, y: 12 });
        expect(tabAnchorOffset(document.createElement("div"))).toBeNull();
        expect(tabAnchorOffset(null)).toBeNull();
    });
});
