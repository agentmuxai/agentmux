// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import { DORMANT_SLOT_ATTR, HIDDEN_TAB_ATTR } from "./pane-overlay";
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

    test("returns false inside a hidden window tab or a dormant pane-stack member", () => {
        for (const attr of [HIDDEN_TAB_ATTR, DORMANT_SLOT_ATTR]) {
            const container = track(makeEl({}));
            container.setAttribute(attr, "");
            const el = makeEl({});
            container.appendChild(el);
            expect(isOverlayElementVisible(el)).toBe(false);
            container.removeAttribute(attr);
            expect(isOverlayElementVisible(el)).toBe(true);
        }
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
