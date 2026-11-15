// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The toast stack cuts a hole through a browser pane's native window it floats over, on every platform.
// `data-pane-overlay` alone does that on Windows only (pane-overlay-auto.ts), so on macOS and Linux the
// stack has to register itself: a toast over a browser pane was drawn under the pane's window.

import { setNotifications } from "@/app/store/flash-notifications";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));

import { __overlayRectCount } from "@/app/platform/pane-overlay";
import { NotificationBubbles } from "./notificationbubbles";

const toast = (id: string) => ({ id, icon: "bell", title: id, message: "", timestamp: "" });

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    // jsdom lays nothing out: give every element a real box, so the stack has an area to cut.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
        left: 600, top: 500, width: 360, height: 120, x: 600, y: 500, right: 960, bottom: 620, toJSON: () => ({}),
    } as DOMRect);
});

afterEach(() => {
    cleanup();
    setNotifications([]);
    document.body.innerHTML = "";
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

describe("NotificationBubbles over a browser pane", () => {
    it("registers a hole while there is a stack, and drops it with the last toast", () => {
        const before = __overlayRectCount();
        setNotifications([toast("a")]);
        render(() => <NotificationBubbles />);
        expect(document.querySelector(".notification-bubbles")).not.toBeNull();
        expect(__overlayRectCount()).toBe(before + 1);

        setNotifications([]);
        expect(document.querySelector(".notification-bubbles")).toBeNull();
        expect(__overlayRectCount()).toBe(before);
    });

    it("registers one hole for the stack, however many toasts it holds", () => {
        const before = __overlayRectCount();
        setNotifications([toast("a"), toast("b"), toast("c")]);
        render(() => <NotificationBubbles />);
        expect(__overlayRectCount()).toBe(before + 1);
    });

    it("leaves no hole behind when it is unmounted with toasts showing", () => {
        const before = __overlayRectCount();
        setNotifications([toast("a")]);
        const { unmount } = render(() => <NotificationBubbles />);
        unmount();
        expect(__overlayRectCount()).toBe(before);
    });
});
