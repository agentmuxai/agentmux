// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { isPrimaryButtonDown, onPrimaryButtonRelease } from "./pointer-drag-state";

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

    // reagentx P2 on PR #3470. Gating hover on isPrimaryButtonDown() is only
    // half a rule: the gate drops the mouseenter that arrived mid-drag, and if
    // the drag ENDS with the cursor still on that anchor no further
    // mouseenter/mouseleave ever fires, so the overlay stays shut. Consumers
    // resync off this notification.
    describe("onPrimaryButtonRelease", () => {
        it("fires on pointerup, after the flag is already false", () => {
            let seen: boolean | undefined;
            const off = onPrimaryButtonRelease(() => {
                seen = isPrimaryButtonDown();
            });
            window.dispatchEvent(pointerEvent("pointerdown", 0));
            window.dispatchEvent(pointerEvent("pointerup", 0));
            // Not merely "was called": a consumer re-checks hover here and
            // would re-suppress itself if the flag were still true.
            expect(seen).toBe(false);
            off();
        });

        it("fires on pointercancel too — an aborted drag still leaves the cursor somewhere", () => {
            let calls = 0;
            const off = onPrimaryButtonRelease(() => calls++);
            window.dispatchEvent(pointerEvent("pointerdown", 0));
            window.dispatchEvent(pointerEvent("pointercancel", 0));
            expect(calls).toBe(1);
            off();
        });

        it("ignores a non-primary release", () => {
            let calls = 0;
            const off = onPrimaryButtonRelease(() => calls++);
            window.dispatchEvent(pointerEvent("pointerup", 2));
            expect(calls).toBe(0);
            off();
        });

        it("does NOT fire on window blur", () => {
            // The flag resets there (we may never see the pointerup), but the
            // window has just lost focus and `:hover` can still match — a
            // resync would pop an overlay onto a background window.
            let calls = 0;
            const off = onPrimaryButtonRelease(() => calls++);
            window.dispatchEvent(pointerEvent("pointerdown", 0));
            window.dispatchEvent(new Event("blur"));
            expect(isPrimaryButtonDown()).toBe(false);
            expect(calls).toBe(0);
            off();
        });

        it("stops firing once unsubscribed", () => {
            let calls = 0;
            const off = onPrimaryButtonRelease(() => calls++);
            window.dispatchEvent(pointerEvent("pointerup", 0));
            expect(calls).toBe(1);
            off();
            window.dispatchEvent(pointerEvent("pointerup", 0));
            expect(calls).toBe(1);
        });

        it("does not call a listener that an earlier listener unsubscribed", () => {
            // A component unmounting a child during its own teardown does
            // exactly this. Dispatch iterates the live Set, so an entry
            // removed before it is reached is never visited — which is the
            // point: an unsubscribed listener must not fire. Taking a
            // defensive copy of the Set first would call `b` anyway.
            const calls: string[] = [];
            const offB = () => offBImpl();
            let offBImpl: () => void = () => {};
            const offA = onPrimaryButtonRelease(() => {
                calls.push("a");
                offB();
            });
            offBImpl = onPrimaryButtonRelease(() => calls.push("b"));
            window.dispatchEvent(pointerEvent("pointerup", 0));
            expect(calls).toEqual(["a"]);
            offA();
        });

        it("still calls the remaining listeners when one unsubscribes ITSELF", () => {
            const calls: string[] = [];
            let offA: () => void = () => {};
            offA = onPrimaryButtonRelease(() => {
                calls.push("a");
                offA();
            });
            const offB = onPrimaryButtonRelease(() => calls.push("b"));
            window.dispatchEvent(pointerEvent("pointerup", 0));
            expect(calls).toEqual(["a", "b"]);
            offB();
        });
    });
});
