// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));

import { __overlayRectCount, overlayCoversPane, registerPaneOverlay } from "./pane-overlay";
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
        const h = registerPaneOverlay(makeEl({ left: 150, top: 150, width: 100, height: 100 }));
        expect(overlayCoversPane("pane-a")).toBe(true);
        h.release();
        expect(overlayCoversPane("pane-a")).toBe(false);
    });

    test("an overlay elsewhere, or a pane with no rect, is not covered", () => {
        registerPaneRect("pane-a", { x: 0, y: 0, w: 100, h: 100 });
        const h = registerPaneOverlay(makeEl({ left: 300, top: 300, width: 50, height: 50 }));
        expect(overlayCoversPane("pane-a")).toBe(false);
        expect(overlayCoversPane("no-such-pane")).toBe(false);
        h.release();
    });
});
