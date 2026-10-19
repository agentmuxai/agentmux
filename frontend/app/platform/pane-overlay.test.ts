// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));

import {
    DORMANT_SLOT_ATTR,
    HIDDEN_TAB_ATTR,
    __overlayRectCount,
    __setAutoOverlayRect,
    isInHiddenPaneTab,
    onPaneOverlayReleased,
    overlayCoversPane,
    registerPaneOverlay,
    remeasurePaneOverlays,
} from "./pane-overlay";
import { __resetPaneRectRegistry, registerPaneRect } from "./pane-rect-registry";

// registerPaneOverlay — the imperative sibling of usePaneOverlay used by the
// plain-DOM context menu (SPEC_MACOS_BROWSER_PANE_CONTEXT_MENU_2026_09_24).

function makeEl(rect = { left: 10, top: 20, width: 100, height: 50 }): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () =>
        ({ ...rect, x: rect.left, y: rect.top, right: rect.left + rect.width, bottom: rect.top + rect.height, toJSON: () => ({}) }) as DOMRect;
    document.body.appendChild(el);
    return el;
}

describe("registerPaneOverlay", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    test("a visible element registers a rect; release drops it (idempotent)", () => {
        const before = __overlayRectCount();
        const h = registerPaneOverlay(makeEl());
        expect(__overlayRectCount()).toBe(before + 1);
        h.release();
        h.release();
        expect(__overlayRectCount()).toBe(before);
    });

    test("visibility:hidden registers nothing until revealed", () => {
        const before = __overlayRectCount();
        const el = makeEl();
        el.style.visibility = "hidden";
        const h = registerPaneOverlay(el);
        expect(__overlayRectCount()).toBe(before);
        el.style.visibility = "";
        h.update();
        expect(__overlayRectCount()).toBe(before + 1);
        h.release();
    });

    test("a zero-size (display:none) element registers nothing", () => {
        const before = __overlayRectCount();
        const h = registerPaneOverlay(makeEl({ left: 0, top: 0, width: 0, height: 0 }));
        expect(__overlayRectCount()).toBe(before);
        h.release();
    });

    test("update after release does not re-register", () => {
        const before = __overlayRectCount();
        const h = registerPaneOverlay(makeEl());
        h.release();
        h.update();
        expect(__overlayRectCount()).toBe(before);
    });
});

// A hidden window tab or dormant pane-stack member keeps its real rect, so an
// overlay inside one must cut no hole through the panes on screen
// (REPORT_BROWSER_PANE_GREY_HOLE_RCA_2026_10_18.md).
describe("overlays in hidden tabs and dormant pane-stack members", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    // The marker watcher is a MutationObserver: its callback runs as a microtask.
    const flushObservers = () => new Promise((resolve) => setTimeout(resolve, 0));

    function makeElIn(container: HTMLElement): HTMLElement {
        const el = makeEl();
        container.appendChild(el);
        return el;
    }

    for (const attr of [HIDDEN_TAB_ATTR, DORMANT_SLOT_ATTR]) {
        test(`an overlay under [${attr}] registers no rect`, () => {
            const container = document.createElement("div");
            container.setAttribute(attr, "");
            document.body.appendChild(container);
            const el = makeElIn(container);
            expect(isInHiddenPaneTab(el)).toBe(true);
            const before = __overlayRectCount();
            const h = registerPaneOverlay(el);
            expect(__overlayRectCount()).toBe(before);
            h.release();
        });

        test(`hiding the container ([${attr}] added) drops the rect, showing it again restores it`, async () => {
            const container = document.createElement("div");
            document.body.appendChild(container);
            const before = __overlayRectCount();
            const h = registerPaneOverlay(makeElIn(container));
            expect(__overlayRectCount()).toBe(before + 1);

            container.setAttribute(attr, "");
            await flushObservers();
            expect(__overlayRectCount()).toBe(before);

            container.removeAttribute(attr);
            await flushObservers();
            expect(__overlayRectCount()).toBe(before + 1);
            h.release();
        });
    }

    test("remeasurePaneOverlays re-reads every registered overlay", () => {
        const container = document.createElement("div");
        document.body.appendChild(container);
        const el = makeElIn(container);
        const h = registerPaneOverlay(el, { keys: true });
        registerPaneRect("pane-r", { x: 0, y: 0, w: 500, h: 500 });
        expect(overlayCoversPane("pane-r")).toBe(true);
        // Set without letting the observer run: only an explicit re-measure sees it.
        container.setAttribute(HIDDEN_TAB_ATTR, "");
        remeasurePaneOverlays();
        expect(overlayCoversPane("pane-r")).toBe(false);
        h.release();
        __resetPaneRectRegistry();
    });
});

describe("isRegisteredPaneOverlay", () => {
    test("true while registered, false after release", async () => {
        const { isRegisteredPaneOverlay } = await import("./pane-overlay");
        const el = makeEl();
        expect(isRegisteredPaneOverlay(el)).toBe(false);
        const h = registerPaneOverlay(el);
        expect(isRegisteredPaneOverlay(el)).toBe(true);
        h.release();
        expect(isRegisteredPaneOverlay(el)).toBe(false);
    });
});

describe("overlayCoversPane", () => {
    afterEach(() => {
        document.body.innerHTML = "";
        __resetPaneRectRegistry();
    });

    test("true while an open overlay intersects the pane, false once released", () => {
        registerPaneRect("pane-a", { x: 0, y: 0, w: 200, h: 200 });
        expect(overlayCoversPane("pane-a")).toBe(false);
        const h = registerPaneOverlay(makeEl({ left: 150, top: 150, width: 100, height: 100 }), { keys: true });
        expect(overlayCoversPane("pane-a")).toBe(true);
        h.release();
        expect(overlayCoversPane("pane-a")).toBe(false);
    });

    test("an overlay elsewhere, or a pane with no rect, is not covered", () => {
        registerPaneRect("pane-a", { x: 0, y: 0, w: 100, h: 100 });
        const h = registerPaneOverlay(makeEl({ left: 300, top: 300, width: 50, height: 50 }), { keys: true });
        expect(overlayCoversPane("pane-a")).toBe(false);
        expect(overlayCoversPane("no-such-pane")).toBe(false);
        h.release();
    });

    test("an overlay that only clips (a tooltip, the drag hole) does not hold the keyboard, and its release tells no one", () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        const cb = vi.fn();
        const off = onPaneOverlayReleased(cb);
        registerPaneRect("pane-a", { x: 0, y: 0, w: 200, h: 200 });
        const h = registerPaneOverlay(makeEl({ left: 10, top: 10, width: 50, height: 50 }));
        expect(overlayCoversPane("pane-a")).toBe(false);
        h.release();
        frames.splice(0).forEach((f) => f(0));
        expect(cb).not.toHaveBeenCalled();
        off();
        vi.unstubAllGlobals();
    });

    test("a data-pane-overlay (auto) element, such as a loading cover or tooltip, does not count", () => {
        registerPaneRect("pane-a", { x: 0, y: 0, w: 200, h: 200 });
        const el = makeEl();
        __setAutoOverlayRect(el, { x: 0, y: 0, w: 200, h: 200 });
        expect(overlayCoversPane("pane-a")).toBe(false);
    });

    test("releasing an overlay that covered a pane tells the listeners once; one elsewhere does not", async () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        const cb = vi.fn();
        const off = onPaneOverlayReleased(cb);
        registerPaneRect("pane-a", { x: 0, y: 0, w: 100, h: 100 });
        const away = registerPaneOverlay(makeEl({ left: 300, top: 300, width: 50, height: 50 }), { keys: true });
        const over = registerPaneOverlay(makeEl({ left: 10, top: 10, width: 50, height: 50 }), { keys: true });
        frames.splice(0).forEach((f) => f(0));
        away.release();
        frames.splice(0).forEach((f) => f(0));
        expect(cb).not.toHaveBeenCalled();
        over.release();
        over.release();
        frames.splice(0).forEach((f) => f(0));
        expect(cb).toHaveBeenCalledTimes(1);
        off();
        vi.unstubAllGlobals();
    });
});
