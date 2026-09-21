// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { isPrimaryButtonDown } from "./pointer-drag-state";

function pointerEvent(type: string, button = 0): PointerEvent {
    return new PointerEvent(type, { button, bubbles: true, cancelable: true });
}

describe("pointer-drag-state", () => {
    afterEach(() => {
        // Reset module singleton state between tests.
        window.dispatchEvent(pointerEvent("pointerup"));
    });

    it("is false before any pointer activity", () => {
        expect(isPrimaryButtonDown()).toBe(false);
    });

    it("becomes true on a primary-button pointerdown", () => {
        window.dispatchEvent(pointerEvent("pointerdown", 0));
        expect(isPrimaryButtonDown()).toBe(true);
    });

    it("ignores a non-primary-button pointerdown", () => {
        window.dispatchEvent(pointerEvent("pointerdown", 2));
        expect(isPrimaryButtonDown()).toBe(false);
    });

    it("becomes false again on pointerup", () => {
        window.dispatchEvent(pointerEvent("pointerdown", 0));
        expect(isPrimaryButtonDown()).toBe(true);
        window.dispatchEvent(pointerEvent("pointerup", 0));
        expect(isPrimaryButtonDown()).toBe(false);
    });

    it("becomes false on pointercancel", () => {
        window.dispatchEvent(pointerEvent("pointerdown", 0));
        window.dispatchEvent(pointerEvent("pointercancel", 0));
        expect(isPrimaryButtonDown()).toBe(false);
    });

    it("resets on window blur (mouse released outside the window)", () => {
        window.dispatchEvent(pointerEvent("pointerdown", 0));
        expect(isPrimaryButtonDown()).toBe(true);
        window.dispatchEvent(new Event("blur"));
        expect(isPrimaryButtonDown()).toBe(false);
    });

    it("still fires on an element that stops propagation (capture phase)", () => {
        const el = document.createElement("div");
        document.body.appendChild(el);
        el.addEventListener("pointerdown", (e) => e.stopPropagation());
        el.dispatchEvent(pointerEvent("pointerdown", 0));
        expect(isPrimaryButtonDown()).toBe(true);
        document.body.removeChild(el);
    });
});
