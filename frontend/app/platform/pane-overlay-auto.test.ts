// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import { isOverlayElementVisible } from "./pane-overlay-auto";

function makeEl(style: Partial<CSSStyleDeclaration>): HTMLDivElement {
    const el = document.createElement("div");
    Object.assign(el.style, style);
    document.body.appendChild(el);
    return el;
}

describe("isOverlayElementVisible", () => {
    const created: HTMLElement[] = [];

    afterEach(() => {
        for (const el of created) el.remove();
        created.length = 0;
    });

    function track(el: HTMLElement): HTMLElement {
        created.push(el);
        return el;
    }

    test("returns true for a default visible element", () => {
        const el = track(makeEl({}));
        expect(isOverlayElementVisible(el)).toBe(true);
    });

    test("returns false when visibility is hidden", () => {
        const el = track(makeEl({ visibility: "hidden" }));
        expect(isOverlayElementVisible(el)).toBe(false);
    });

    test("returns false when display is none", () => {
        const el = track(makeEl({ display: "none" }));
        expect(isOverlayElementVisible(el)).toBe(false);
    });

    test("returns false when opacity is 0", () => {
        const el = track(makeEl({ opacity: "0" }));
        expect(isOverlayElementVisible(el)).toBe(false);
    });

    test("returns true when opacity is non-zero (e.g., 0.5)", () => {
        const el = track(makeEl({ opacity: "0.5" }));
        expect(isOverlayElementVisible(el)).toBe(true);
    });

    test("returns true for opacity 1", () => {
        const el = track(makeEl({ opacity: "1" }));
        expect(isOverlayElementVisible(el)).toBe(true);
    });
});
