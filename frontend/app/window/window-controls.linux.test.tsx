// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Linux window controls: GNOME-style buttons placed by the desktop's button
 * layout, Restore while maximized, dimmed while unfocused.
 * docs/specs/SPEC_LINUX_WINDOW_CONTROLS_GNOME_STYLE_2026_10_16.md §5
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    maximized: false,
    calls: [] as string[],
    layout: ":minimize,maximize,close" as string | null,
}));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        minimizeWindow: () => h.calls.push("minimize"),
        maximizeWindow: () => h.calls.push("maximize"),
        closeWindow: async () => {
            h.calls.push("close");
        },
        windows: {
            isMaximized: async (label: string) => {
                h.calls.push(`isMaximized:${label}`);
                return h.maximized;
            },
        },
    }),
}));
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({ getWindowButtonLayout: async () => h.layout }),
}));
vi.mock("@/app/hook/useWindowDrag.platform", () => ({ useWindowDrag: () => ({ dragProps: {} }) }));

import {
    DEFAULT_WINDOW_BUTTON_LAYOUT,
    loadWindowButtonLayout,
    parseWindowButtonLayout,
    resetWindowButtonLayoutForTest,
} from "./window-button-layout";
import { MAXIMIZED_RESIZE_SETTLE_MS, WindowControlsLeft, WindowControlsRight } from "./window-controls.linux";

const ids = (root: ParentNode) =>
    [...root.querySelectorAll("[data-testid^=window-]")].map((b) => b.getAttribute("data-testid"));

beforeEach(() => {
    h.maximized = false;
    h.calls = [];
    h.layout = ":minimize,maximize,close";
    resetWindowButtonLayoutForTest();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("parseWindowButtonLayout", () => {
    it("splits left and right of the colon, in order", () => {
        expect(parseWindowButtonLayout(":minimize,maximize,close")).toEqual({
            left: [],
            right: ["minimize", "maximize", "close"],
        });
        expect(parseWindowButtonLayout("close,minimize,maximize:")).toEqual({
            left: ["close", "minimize", "maximize"],
            right: [],
        });
        expect(parseWindowButtonLayout("close:maximize")).toEqual({ left: ["close"], right: ["maximize"] });
    });

    it("drops names the app doesn't draw, repeats and whitespace", () => {
        expect(parseWindowButtonLayout("appmenu: minimize , spacer,close,close,icon")).toEqual({
            left: [],
            right: ["minimize", "close"],
        });
        expect(parseWindowButtonLayout("appmenu:close")).toEqual({ left: [], right: ["close"] });
    });

    it("always keeps Close", () => {
        expect(parseWindowButtonLayout("appmenu:")).toEqual({ left: [], right: ["close"] });
        expect(parseWindowButtonLayout("minimize:")).toEqual({ left: ["minimize"], right: ["close"] });
    });

    it("gives the default for nothing to read", () => {
        expect(parseWindowButtonLayout(null)).toBe(DEFAULT_WINDOW_BUTTON_LAYOUT);
        expect(parseWindowButtonLayout(undefined)).toBe(DEFAULT_WINDOW_BUTTON_LAYOUT);
        expect(parseWindowButtonLayout("  ")).toBe(DEFAULT_WINDOW_BUTTON_LAYOUT);
    });
});

describe("Linux window controls", () => {
    it("default: the three buttons on the right in order, none on the left", () => {
        const right = render(() => <WindowControlsRight />).container;
        const left = render(() => <WindowControlsLeft />).container;
        expect(ids(right)).toEqual(["window-minimize-btn", "window-maximize-btn", "window-close-btn"]);
        expect(ids(left)).toEqual([]);
    });

    it("follows the desktop's layout once the host answers", async () => {
        h.layout = "close,minimize,maximize:";
        await loadWindowButtonLayout();
        const right = render(() => <WindowControlsRight />).container;
        const left = render(() => <WindowControlsLeft />).container;
        expect(ids(left)).toEqual(["window-close-btn", "window-minimize-btn", "window-maximize-btn"]);
        expect(ids(right)).toEqual([]);
    });

    it("an unreadable layout keeps the default", async () => {
        h.layout = null;
        await loadWindowButtonLayout();
        const right = render(() => <WindowControlsRight />).container;
        expect(ids(right)).toEqual(["window-minimize-btn", "window-maximize-btn", "window-close-btn"]);
    });

    it("clicks call the window's minimize, maximize and close", () => {
        const c = render(() => <WindowControlsRight />).container;
        for (const id of ["minimize", "maximize", "close"]) {
            (c.querySelector(`[data-testid=window-${id}-btn]`) as HTMLButtonElement).click();
        }
        expect(h.calls.filter((x) => !x.startsWith("isMaximized"))).toEqual(["minimize", "maximize", "close"]);
    });

    it("shows Restore while the window is maximized, and follows a resize", async () => {
        h.maximized = true;
        const c = render(() => <WindowControlsRight />).container;
        const btn = () => c.querySelector("[data-testid=window-maximize-btn]")!;
        await vi.waitFor(() => expect(btn().getAttribute("title")).toBe("Restore"));
        expect(h.calls).toContain("isMaximized:main");

        vi.useFakeTimers();
        h.maximized = false;
        window.dispatchEvent(new Event("resize"));
        vi.advanceTimersByTime(MAXIMIZED_RESIZE_SETTLE_MS);
        vi.useRealTimers();
        await vi.waitFor(() => expect(btn().getAttribute("title")).toBe("Maximize"));
    });

    it("dims while the window is not focused", () => {
        const c = render(() => <WindowControlsRight />).container;
        const group = c.querySelector(".gnome-window-buttons")!;
        window.dispatchEvent(new Event("blur"));
        expect(group.classList.contains("window-unfocused")).toBe(true);
        window.dispatchEvent(new Event("focus"));
        expect(group.classList.contains("window-unfocused")).toBe(false);
    });
});
