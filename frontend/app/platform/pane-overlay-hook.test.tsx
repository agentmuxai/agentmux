// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An overlay rendered through a Portal sits under document.body, outside its
// tab's DOM, where the hidden-tab marker can't be seen. usePaneOverlay reads
// the window-tab context instead, and a Portal-mounted data-pane-overlay marks
// itself with useHiddenTabMarker. REPORT_BROWSER_PANE_GREY_HOLE_RCA_2026_10_18.md.

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { Portal } from "solid-js/web";
import { afterEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));

import { WindowTabDisplayedProvider } from "@/app/workspace/window-tab-visibility";
import { __autoOverlayRectCount, __overlayRectCount, useHiddenTabMarker, usePaneOverlay } from "./pane-overlay";
import { startPaneOverlayAutoService } from "./pane-overlay-auto";

function makeEl(): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () =>
        ({ left: 10, top: 20, width: 100, height: 50, x: 10, y: 20, right: 110, bottom: 70, toJSON: () => ({}) }) as DOMRect;
    // Attached outside the rendered tree, like a Portal's mount point.
    document.body.appendChild(el);
    return el;
}

function Overlay(p: { el: HTMLElement }) {
    usePaneOverlay(() => p.el);
    return null;
}

describe("usePaneOverlay and the window tab's displayed state", () => {
    afterEach(() => {
        cleanup();
        document.body.innerHTML = "";
    });

    test("no hole while the tab is hidden; the hole appears when it is shown and goes when hidden again", () => {
        const [displayed, setDisplayed] = createSignal(false);
        const before = __overlayRectCount();
        const el = makeEl();
        render(() => (
            <WindowTabDisplayedProvider value={displayed}>
                <Overlay el={el} />
            </WindowTabDisplayedProvider>
        ));
        expect(__overlayRectCount()).toBe(before);

        setDisplayed(true);
        expect(__overlayRectCount()).toBe(before + 1);

        setDisplayed(false);
        expect(__overlayRectCount()).toBe(before);
    });

    test("a Portal-mounted data-pane-overlay (tooltip, menu) drops its auto-discovered hole while its window tab is hidden", async () => {
        // The auto service runs on Windows only, and needs ResizeObserver and a real box.
        const ua = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
        vi.stubGlobal(
            "ResizeObserver",
            class {
                observe(): void {}
                unobserve(): void {}
                disconnect(): void {}
            },
        );
        const box = vi
            .spyOn(HTMLElement.prototype, "getBoundingClientRect")
            .mockReturnValue({ left: 10, top: 20, width: 100, height: 50, x: 10, y: 20, right: 110, bottom: 70, toJSON: () => ({}) } as DOMRect);
        const flushObservers = () => new Promise((resolve) => setTimeout(resolve, 0));
        startPaneOverlayAutoService();

        function PortalOverlay() {
            const hiddenTab = useHiddenTabMarker();
            return (
                <Portal>
                    <div data-pane-overlay data-window-tab-hidden={hiddenTab()} />
                </Portal>
            );
        }
        const [displayed, setDisplayed] = createSignal(true);
        const before = __autoOverlayRectCount();
        render(() => (
            <WindowTabDisplayedProvider value={displayed}>
                <PortalOverlay />
            </WindowTabDisplayedProvider>
        ));
        await flushObservers();
        expect(__autoOverlayRectCount()).toBe(before + 1);

        setDisplayed(false);
        await flushObservers();
        expect(__autoOverlayRectCount()).toBe(before);

        setDisplayed(true);
        await flushObservers();
        expect(__autoOverlayRectCount()).toBe(before + 1);

        ua.mockRestore();
        box.mockRestore();
        vi.unstubAllGlobals();
    });

    test("outside any window tab (a floating window) the overlay always clips", () => {
        const before = __overlayRectCount();
        const { unmount } = render(() => <Overlay el={makeEl()} />);
        expect(__overlayRectCount()).toBe(before + 1);
        unmount();
        expect(__overlayRectCount()).toBe(before);
    });
});
