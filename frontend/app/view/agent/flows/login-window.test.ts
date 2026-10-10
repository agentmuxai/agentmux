// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The login window reserved at the click (login-window.ts): a browser host's
 * window is used for the URL that comes later, closed when it isn't, and a
 * host that reserves nothing (the desktop) keeps its system-browser path.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    createBlock: vi.fn(),
    invokeCommand: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({ createBlock: hub.createBlock }));
vi.mock("@/app/platform/ipc", () => ({ invokeCommand: hub.invokeCommand }));

import { releaseLoginWindow, reserveLoginWindow, takeLoginWindow } from "./login-window";
import { openOAuthBrowserPane } from "./open-oauth-pane";
import { installCefWireHost } from "../../../../test/cef-wire-host";

installCefWireHost();

const URL = "https://claude.ai/oauth/authorize?client_id=abc";

/** A browser host's window: records what happened to it. */
function fakeWindow() {
    return { navigate: vi.fn(), close: vi.fn() };
}

let reserve: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
    hub.createBlock.mockReset();
    hub.invokeCommand.mockReset();
    reserve = vi.spyOn(window.api, "reserveExternalWindow");
});
afterEach(() => {
    releaseLoginWindow();
    vi.restoreAllMocks();
    vi.useRealTimers();
});

describe("the login window", () => {
    it("on a browser host, the window reserved at the click gets the URL, and the system browser isn't asked", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        expect(await openOAuthBrowserPane(URL)).toBe("external");
        expect(win.navigate).toHaveBeenCalledWith(URL);
        expect(win.close).not.toHaveBeenCalled();
        expect(hub.invokeCommand).not.toHaveBeenCalled();
    });

    it("is handed over once: a second URL goes the usual way", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        await openOAuthBrowserPane(URL);
        hub.invokeCommand.mockResolvedValue(undefined);
        await openOAuthBrowserPane(URL);
        expect(win.navigate).toHaveBeenCalledTimes(1);
        expect(hub.invokeCommand).toHaveBeenCalledWith("open_external", { url: URL });
    });

    it("is closed when the login ends without using it", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        releaseLoginWindow();
        expect(win.close).toHaveBeenCalledTimes(1);
        expect(takeLoginWindow()).toBeNull();
    });

    it("closes an unused one when a new login reserves", () => {
        const first = fakeWindow();
        const second = fakeWindow();
        reserve.mockReturnValueOnce(first).mockReturnValueOnce(second);
        reserveLoginWindow();
        reserveLoginWindow();
        expect(first.close).toHaveBeenCalledTimes(1);
        expect(takeLoginWindow()).toBe(second);
    });

    it("isn't used once it's stale", () => {
        vi.useFakeTimers();
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        vi.advanceTimersByTime(5 * 60_000 + 1);
        expect(takeLoginWindow()).toBeNull();
        expect(win.close).toHaveBeenCalledTimes(1);
    });

    it("on the desktop (nothing reserved), the system browser opens as before", async () => {
        hub.invokeCommand.mockResolvedValue(undefined);
        reserveLoginWindow();
        expect(window.api.reserveExternalWindow()).toBeNull();
        expect(await openOAuthBrowserPane(URL)).toBe("external");
        expect(hub.invokeCommand).toHaveBeenCalledWith("open_external", { url: URL });
    });

    it("a host that throws while reserving leaves the usual path", async () => {
        reserve.mockImplementation(() => {
            throw new Error("blocked");
        });
        hub.invokeCommand.mockResolvedValue(undefined);
        expect(() => reserveLoginWindow()).not.toThrow();
        expect(await openOAuthBrowserPane(URL)).toBe("external");
        expect(hub.invokeCommand).toHaveBeenCalled();
    });
});
