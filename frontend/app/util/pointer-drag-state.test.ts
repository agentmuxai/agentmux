// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { isPrimaryButtonDown, isRestingAtPress, onPrimaryButtonRelease } from "./pointer-drag-state";

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

    // A click that redraws its control puts a new element under a pointer that has not moved, and the
    // browser reports it as entered. Hover panels ask this before they open for such an enter.
    describe("isRestingAtPress", () => {
        const at = (type: string, clientX: number, clientY: number, more: PointerEventInit = {}): PointerEvent =>
            new PointerEvent(type, { clientX, clientY, bubbles: true, ...more });

        afterEach(() => {
            window.dispatchEvent(new Event("blur"));
        });

        it("is false before any press", () => {
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(false);
        });

        it("is true where the mouse was pressed, through the release and a few pixels of drift", () => {
            window.dispatchEvent(at("pointerdown", 100, 20));
            window.dispatchEvent(at("pointerup", 100, 20));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(true);
            window.dispatchEvent(at("pointermove", 103, 18));
            expect(isRestingAtPress({ clientX: 103, clientY: 18 })).toBe(true);
        });

        it("is false for an event somewhere else, though no move has been seen yet", () => {
            window.dispatchEvent(at("pointerdown", 100, 20));
            expect(isRestingAtPress({ clientX: 180, clientY: 20 })).toBe(false);
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(true);
        });

        it("ends once the pointer has moved away, even when it comes back to the same point", () => {
            window.dispatchEvent(at("pointerdown", 100, 20));
            window.dispatchEvent(at("pointermove", 140, 60));
            window.dispatchEvent(at("pointermove", 100, 20));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(false);
        });

        it("ends when the wheel is turned: what the wheel brings under the pointer is hovered like anything else", () => {
            window.dispatchEvent(at("pointerdown", 100, 20));
            window.dispatchEvent(at("pointerup", 100, 20));
            window.dispatchEvent(new WheelEvent("wheel", { clientX: 100, clientY: 20, deltaY: 40, bubbles: true }));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(false);
        });

        it("counts a press with any mouse button, and none by touch", () => {
            window.dispatchEvent(at("pointerdown", 100, 20, { button: 2 }));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(true);
            window.dispatchEvent(at("pointerdown", 100, 20, { pointerType: "touch" }));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(false);
        });

        it("ends when the window loses focus", () => {
            window.dispatchEvent(at("pointerdown", 100, 20));
            window.dispatchEvent(new Event("blur"));
            expect(isRestingAtPress({ clientX: 100, clientY: 20 })).toBe(false);
        });
    });

    // The list of a native select is a window of its own: the page hears the press and nothing after it,
    // neither a pointerup nor a blur. Hover panels stayed shut in the whole window until the next click.
    describe("a release the page never heard", () => {
        const mouse = (type: string, buttons: number, pointerType = "mouse"): PointerEvent =>
            new PointerEvent(type, { button: type === "pointerdown" ? 0 : -1, buttons, pointerType, bubbles: true });

        it("is noticed on the next mouse move that reports no button", () => {
            window.dispatchEvent(mouse("pointerdown", 1));
            expect(isPrimaryButtonDown()).toBe(true);
            window.dispatchEvent(mouse("pointermove", 0));
            expect(isPrimaryButtonDown()).toBe(false);
        });

        it("is noticed on an element's enter, which comes before the move and is what a hover panel decides on", () => {
            const el = document.body.appendChild(document.createElement("div"));
            let seenByTheElement: boolean | undefined;
            el.addEventListener("pointerover", () => {
                seenByTheElement = isPrimaryButtonDown();
            });
            window.dispatchEvent(mouse("pointerdown", 1));
            el.dispatchEvent(mouse("pointerover", 0));
            expect(seenByTheElement).toBe(false);
            el.remove();
        });

        it("tells the release listeners once, so a panel whose anchor is under the pointer can open", () => {
            let calls = 0;
            const off = onPrimaryButtonRelease(() => {
                calls++;
            });
            window.dispatchEvent(mouse("pointerdown", 1));
            window.dispatchEvent(mouse("pointermove", 0));
            window.dispatchEvent(mouse("pointermove", 0));
            expect(calls).toBe(1);
            off();
        });

        it("leaves a drag alone: a move with the button still down, or with only another button down", () => {
            window.dispatchEvent(mouse("pointerdown", 1));
            window.dispatchEvent(mouse("pointermove", 1));
            window.dispatchEvent(mouse("pointermove", 3));
            expect(isPrimaryButtonDown()).toBe(true);
            window.dispatchEvent(mouse("pointermove", 2));
            expect(isPrimaryButtonDown()).toBe(false);
        });

        it("believes only a mouse about a mouse: a finger that is down is not lifted by a mouse move", () => {
            window.dispatchEvent(mouse("pointerdown", 1, "touch"));
            window.dispatchEvent(mouse("pointermove", 0));
            expect(isPrimaryButtonDown()).toBe(true);
            window.dispatchEvent(mouse("pointerdown", 1));
            window.dispatchEvent(mouse("pointermove", 0, "pen"));
            expect(isPrimaryButtonDown()).toBe(true);
        });
    });
});
