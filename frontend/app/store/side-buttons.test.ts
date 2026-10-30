// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The window listener, not just the rules: what a real side-button press delivers to a page.
 * Chromium drops the mousedown and mouseup that follow a cancelled pointerdown, so the
 * release that acts must be the pointer's (found on Linux, where the host has no path of its own).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hooks = vi.hoisted(() => ({
    back: vi.fn(),
    forward: vi.fn(),
    settings: {} as Record<string, unknown>,
    modalOpen: false,
    menuOpen: false,
}));

vi.mock("./pane-history", () => ({ paneHistoryBack: hooks.back, paneHistoryForward: hooks.forward }));
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({ listen: () => Promise.resolve(() => {}), browserPanes: undefined }),
}));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => hooks.settings }));
vi.mock("@/app/host/js-context-menu", () => ({ isJsContextMenuOpen: () => hooks.menuOpen }));
vi.mock("@/app/store/modalmodel", () => ({ modalsModel: { hasOpenModals: () => hooks.modalOpen } }));

import { DEBOUNCE_MS } from "./side-button-rules";
import { installSideButtons } from "./side-buttons";

let uninstall: () => void;
let nowMs = 1_000;

beforeEach(() => {
    hooks.back.mockClear();
    hooks.forward.mockClear();
    hooks.settings = {};
    hooks.modalOpen = false;
    hooks.menuOpen = false;
    nowMs += 10 * DEBOUNCE_MS;
    vi.spyOn(performance, "now").mockImplementation(() => nowMs);
    vi.spyOn(console, "info").mockImplementation(() => {});
    uninstall = installSideButtons();
});

afterEach(() => {
    uninstall();
    vi.restoreAllMocks();
});

/** The events a browser sends for one side-button press, in order, as it sends them. `suppressCompat`
 *  is Chromium's rule: a cancelled pointerdown drops the mousedown and mouseup that would follow. */
function press(button: number, init: MouseEventInit = {}): { cancelled: Record<string, boolean>; delivered: string[] } {
    const target = document.body;
    const cancelled: Record<string, boolean> = {};
    const delivered: string[] = [];
    const fire = (type: string, buttons: number): boolean => {
        const e = new MouseEvent(type, { bubbles: true, cancelable: true, button, buttons, ...init });
        target.dispatchEvent(e);
        delivered.push(type);
        cancelled[type] = e.defaultPrevented;
        return e.defaultPrevented;
    };
    const down = fire("pointerdown", button === 3 ? 8 : 16);
    if (!down) fire("mousedown", button === 3 ? 8 : 16);
    const up = fire("pointerup", 0);
    if (!down && !up) fire("mouseup", 0);
    fire("auxclick", 0);
    return { cancelled, delivered };
}

describe("installSideButtons — the window listener", () => {
    it("goes back on the Back button's release, though Chromium drops the mouse events after a cancelled pointerdown", () => {
        const { cancelled, delivered } = press(3);
        expect(cancelled.pointerdown).toBe(true);
        expect(delivered).not.toContain("mouseup");
        expect(hooks.back).toHaveBeenCalledTimes(1);
        expect(hooks.forward).not.toHaveBeenCalled();
    });

    it("goes forward on the Forward button's release", () => {
        press(4);
        expect(hooks.forward).toHaveBeenCalledTimes(1);
        expect(hooks.back).not.toHaveBeenCalled();
    });

    it("acts once for a press, even if a browser delivers the mouse release too", () => {
        const target = document.body;
        target.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, cancelable: true, button: 3 }));
        target.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 3 }));
        expect(hooks.back).toHaveBeenCalledTimes(1);
    });

    it("acts on the mouse release for a press made while the middle button is held, which has no pointerdown or pointerup", () => {
        const target = document.body;
        target.dispatchEvent(
            new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 3, buttons: 4 | 8 })
        );
        target.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 3, buttons: 4 }));
        expect(hooks.back).toHaveBeenCalledTimes(1);
    });

    it("does nothing on the press itself, only on the release", () => {
        document.body.dispatchEvent(
            new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 3, buttons: 8 })
        );
        expect(hooks.back).not.toHaveBeenCalled();
    });

    it("consumes the press and the release, so the page and Chromium's own navigation never see them", () => {
        const { cancelled } = press(3);
        expect(cancelled.pointerdown).toBe(true);
        expect(cancelled.pointerup).toBe(true);
        expect(cancelled.auxclick).toBe(true);
    });

    it("leaves the other buttons alone", () => {
        const { cancelled } = press(1);
        expect(Object.values(cancelled).every((c) => c === false)).toBe(true);
        expect(hooks.back).not.toHaveBeenCalled();
        expect(hooks.forward).not.toHaveBeenCalled();
    });

    it("does not act with Ctrl, Shift or Alt held, or with the left button down, but still consumes it", () => {
        for (const init of [{ ctrlKey: true }, { shiftKey: true }, { altKey: true }]) {
            nowMs += 10 * DEBOUNCE_MS;
            const { cancelled } = press(3, init);
            expect(cancelled.pointerdown).toBe(true);
        }
        document.body.dispatchEvent(
            new MouseEvent("pointerup", { bubbles: true, cancelable: true, button: 3, buttons: 1 })
        );
        expect(hooks.back).not.toHaveBeenCalled();
    });

    it("does nothing while a modal is open, or when the setting is off, and then leaves the button to the page", () => {
        hooks.modalOpen = true;
        press(3);
        expect(hooks.back).not.toHaveBeenCalled();
        hooks.modalOpen = false;
        hooks.settings = { "mouse:sidebuttons": false };
        nowMs += 10 * DEBOUNCE_MS;
        const { cancelled } = press(3);
        expect(cancelled.pointerdown).toBe(false);
        expect(hooks.back).not.toHaveBeenCalled();
    });

    it("does nothing while a context menu is open, though the press closes it before the release", () => {
        hooks.menuOpen = true;
        const target = document.body;
        target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 3, buttons: 4 | 8 }));
        hooks.menuOpen = false; // the menu's own mousedown listener closed it
        target.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 3, buttons: 4 }));
        expect(hooks.back).not.toHaveBeenCalled();
        // The next press, with no menu, acts as usual.
        nowMs += 10 * DEBOUNCE_MS;
        press(3);
        expect(hooks.back).toHaveBeenCalledTimes(1);
    });

    it("stops listening when uninstalled", () => {
        uninstall();
        press(3);
        expect(hooks.back).not.toHaveBeenCalled();
        uninstall = installSideButtons();
    });
});
