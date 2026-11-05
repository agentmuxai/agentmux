// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The toast stack keeps clear of a block marked data-toast-avoid, including when that block only
// resizes (a composer's textarea growing as you type changes no DOM node).

import { setNotifications } from "@/app/store/flash-notifications";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotificationBubbles } from "./notificationbubbles";
import { naturalHeight } from "./use-toast-placement";

let resizeCallbacks: Array<() => void> = [];
let observed: Element[] = [];
class FakeResizeObserver {
    constructor(cb: () => void) {
        resizeCallbacks.push(cb);
    }
    observe(el: Element) {
        observed.push(el);
    }
    unobserve(el: Element) {
        observed = observed.filter((e) => e !== el);
    }
    disconnect() {
        observed = [];
    }
}

const box = (top: number) => ({ left: 600, top, right: 1000, bottom: 760, width: 400, height: 760 - top, x: 600, y: top, toJSON: () => ({}) });

beforeEach(() => {
    vi.useFakeTimers();
    resizeCallbacks = [];
    observed = [];
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 768, configurable: true });
});

afterEach(() => {
    cleanup();
    setNotifications([]);
    document.body.innerHTML = "";
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

const stackBottom = () => (document.querySelector(".notification-bubbles") as HTMLElement).style.bottom;

describe("NotificationBubbles placement", () => {
    it("sits above a marked block, follows the block when it only resizes, and returns when it goes", () => {
        const block = document.createElement("div");
        block.setAttribute("data-toast-avoid", "");
        let top = 700;
        block.getBoundingClientRect = () => box(top) as DOMRect;
        document.body.appendChild(block);

        setNotifications([{ id: "t", icon: "bell", title: "t", message: "", timestamp: "" }]);
        render(() => <NotificationBubbles />);
        vi.advanceTimersByTime(50); // the first placement (a frame)
        // The stack has no size in jsdom: its bottom edge goes 8 px above the block's top (700).
        expect(stackBottom()).toBe("76px");
        expect(observed).toContain(block);

        // The block grows upward with no DOM mutation: only its ResizeObserver says so.
        top = 650;
        resizeCallbacks.forEach((cb) => cb());
        vi.advanceTimersByTime(250);
        expect(stackBottom()).toBe("126px");

        // The block is removed: the stack goes back to its usual place.
        block.remove();
        window.dispatchEvent(new Event("resize"));
        vi.advanceTimersByTime(250);
        expect(stackBottom()).toBe("34px");
        expect(observed).not.toContain(block);
    });
});

describe("the stack's measured height", () => {
    it("is read with the cap placement put on it lifted, and the cap is put back", () => {
        // A stack shortened to fit above the status bar reports its shortened height; placed from that, the next
        // measure would lift the cap and the stack would grow back, and the two would alternate.
        const stack = document.createElement("div");
        const body = document.createElement("div");
        stack.appendChild(body);
        body.style.maxHeight = "calc(192px / var(--zoomfactor))";
        Object.defineProperty(stack, "offsetHeight", { get: () => (body.style.maxHeight ? 192 : 400) });
        expect(naturalHeight(stack)).toBe(400);
        expect(body.style.maxHeight).toBe("calc(192px / var(--zoomfactor))");
    });

    it("keeps the body's scroll position: with the cap lifted a browser clamps it to 0", () => {
        const stack = document.createElement("div");
        const body = document.createElement("div");
        stack.appendChild(body);
        body.style.maxHeight = "calc(192px / var(--zoomfactor))";
        // A body that can only scroll while capped, as a real one does.
        let scrollTop = 120;
        Object.defineProperty(body, "scrollTop", {
            get: () => (body.style.maxHeight ? scrollTop : 0),
            set: (v: number) => {
                if (body.style.maxHeight) scrollTop = v;
            },
        });
        Object.defineProperty(stack, "offsetHeight", { get: () => (body.style.maxHeight ? 192 : 400) });
        naturalHeight(stack);
        expect(body.scrollTop).toBe(120);
    });
});
