// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { bodyHoldVars, holdsBodyDuringDrag, measureBodyHold } from "./pane-body-hold";

/** An element jsdom does not lay out: give it the inner size and padding a browser would report. */
function box(inner: { w: number; h: number }, padding: string): HTMLElement {
    const el = document.createElement("div");
    el.style.padding = padding;
    Object.defineProperty(el, "clientWidth", { value: inner.w });
    Object.defineProperty(el, "clientHeight", { value: inner.h });
    document.body.append(el);
    return el;
}

describe("measureBodyHold", () => {
    it("is the content box: the inner size less the padding", () => {
        const el = box({ w: 410, h: 305 }, "5px");
        expect(measureBodyHold(el)).toEqual({ width: 400, height: 295 });
        el.remove();
    });

    it("is the whole inner size for a pane with no padding", () => {
        const el = box({ w: 410, h: 305 }, "0");
        expect(measureBodyHold(el)).toEqual({ width: 410, height: 305 });
        el.remove();
    });

    it("is nothing for a box that is not laid out", () => {
        const el = box({ w: 0, h: 0 }, "5px");
        expect(measureBodyHold(el)).toBeNull();
        el.remove();
    });
});

describe("which panes hold", () => {
    it("every view but a browser, whose page is a native window that follows the pane's rect", () => {
        expect(holdsBodyDuringDrag("term")).toBe(true);
        expect(holdsBodyDuringDrag("agent")).toBe(true);
        expect(holdsBodyDuringDrag(undefined)).toBe(true);
        expect(holdsBodyDuringDrag("browser")).toBe(false);
    });
});

describe("bodyHoldVars", () => {
    it("is the size as custom properties, and nothing when there is no hold", () => {
        expect(bodyHoldVars({ width: 400, height: 295.5 })).toEqual({ "--held-width": "400px", "--held-height": "295.5px" });
        expect(bodyHoldVars(null)).toEqual({});
    });
});

describe("the held body's style", () => {
    const scss = readFileSync(resolve(__dirname, "block.scss"), "utf8");

    it("gives the view's place a box of the held size, as a flex container, so the view lays out as it did", () => {
        const at = scss.indexOf("&.block-content--held > .pane-view-host");
        expect(at).toBeGreaterThan(-1);
        const rule = scss.slice(at, scss.indexOf("}", at));
        expect(rule).toContain("display: flex");
        expect(rule).toContain("flex: none");
        expect(rule).toContain("width: var(--held-width)");
        expect(rule).toContain("height: var(--held-height)");
    });

    it("leaves the content box to clip what is past the frame", () => {
        const rule = scss.slice(scss.indexOf(".block-content {"), scss.indexOf("&.block-no-padding"));
        expect(rule).toContain("overflow: hidden");
    });
});
