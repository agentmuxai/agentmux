// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The login window reserved at the click (login-window.ts): a browser host's
 * window is used for the URL that comes later, closed by its owner on every
 * way out when it isn't, never closed by an older flow, and closed by a timer
 * if abandoned. A host that reserves nothing (the desktop) keeps its
 * system-browser path.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    createBlock: vi.fn(),
    invokeCommand: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({ createBlock: hub.createBlock }));
vi.mock("@/app/platform/ipc", () => ({ invokeCommand: hub.invokeCommand }));

import { createRoot, createSignal } from "solid-js";

import { createConnectWindow, reserveLoginWindow, takeLoginWindow, withLoginWindow } from "./login-window";
import { openOAuthBrowserPane } from "./open-oauth-pane";
import { installCefWireHost } from "../../../../test/cef-wire-host";

installCefWireHost();

const URL = "https://claude.ai/oauth/authorize?client_id=abc";
const URL_PROVIDER = { headlessLoginUrlUnsupported: false };

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
    takeLoginWindow();
    vi.restoreAllMocks();
    vi.useRealTimers();
});

describe("the login window", () => {
    it("on a browser host, the window reserved at the click gets the URL, and the system browser isn't asked", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const release = reserveLoginWindow();
        expect(await openOAuthBrowserPane(URL)).toBe("external");
        expect(win.navigate).toHaveBeenCalledWith(URL);
        release(); // the flow ends: the used window stays open
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

    it("is closed by its own release when no URL used it", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const release = reserveLoginWindow();
        release();
        expect(win.close).toHaveBeenCalledTimes(1);
        expect(takeLoginWindow()).toBeNull();
    });

    it("an older flow's release never closes a newer login's window", () => {
        const first = fakeWindow();
        const second = fakeWindow();
        reserve.mockReturnValueOnce(first).mockReturnValueOnce(second);
        const releaseFirst = reserveLoginWindow();
        reserveLoginWindow(); // a new login: the unused first one is closed
        expect(first.close).toHaveBeenCalledTimes(1);
        releaseFirst(); // the first flow ends later
        expect(second.close).not.toHaveBeenCalled();
        expect(takeLoginWindow()).toBe(second);
    });

    it("is closed by a timer if nothing releases or uses it", () => {
        vi.useFakeTimers();
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        vi.advanceTimersByTime(10 * 60_000 - 1);
        expect(win.close).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(win.close).toHaveBeenCalledTimes(1);
        expect(takeLoginWindow()).toBeNull();
    });

    it("a used window isn't closed by the timer", () => {
        vi.useFakeTimers();
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        reserveLoginWindow();
        expect(takeLoginWindow()).toBe(win);
        vi.advanceTimersByTime(11 * 60_000);
        expect(win.close).not.toHaveBeenCalled();
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

describe("withLoginWindow", () => {
    it("reserves before the flow runs, so the reservation is inside the click", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        let reservedFirst = false;
        await withLoginWindow(URL_PROVIDER, async () => {
            reservedFirst = reserve.mock.calls.length === 1;
        });
        expect(reservedFirst).toBe(true);
    });

    it("closes the unused window when the flow returns early, as a failed CLI lookup does", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        await withLoginWindow(URL_PROVIDER, async () => undefined);
        expect(win.close).toHaveBeenCalledTimes(1);
    });

    it("closes it when the flow throws, and passes the error on", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        await expect(
            withLoginWindow(URL_PROVIDER, async () => {
                throw new Error("resolve failed");
            }),
        ).rejects.toThrow("resolve failed");
        expect(win.close).toHaveBeenCalledTimes(1);
    });

    it("leaves the window the flow used open", async () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        await withLoginWindow(URL_PROVIDER, () => openOAuthBrowserPane(URL));
        expect(win.navigate).toHaveBeenCalledWith(URL);
        expect(win.close).not.toHaveBeenCalled();
    });

    it("reserves nothing for a provider whose login runs in a terminal, or no provider", async () => {
        await withLoginWindow({ headlessLoginUrlUnsupported: true }, async () => undefined);
        await withLoginWindow(undefined, async () => undefined);
        expect(reserve).not.toHaveBeenCalled();
    });
});

describe("createConnectWindow (a connect whose URL comes after its start call)", () => {
    /** A connect's state, a real Solid signal, and the tracker in its own root. */
    function setup() {
        const [kind, setKind] = createSignal("unauthenticated");
        let dispose!: () => void;
        const tracker = createRoot((d) => {
            dispose = d;
            return createConnectWindow(kind);
        });
        return { setKind, tracker, dispose };
    }

    it("releases when the state leaves waiting without a URL (cancel, failure): the effect tracks from its first run", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const { setKind, tracker, dispose } = setup();
        const settled = tracker.begin(URL_PROVIDER);
        setKind("waiting");
        settled(); // the start call settles while the URL is still being polled for
        expect(win.close).not.toHaveBeenCalled();
        setKind("unauthenticated"); // cancelled
        expect(win.close).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("keeps the window for a URL that arrives while waiting", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const { setKind, tracker, dispose } = setup();
        const settled = tracker.begin(URL_PROVIDER);
        setKind("waiting");
        settled();
        expect(takeLoginWindow()).toBe(win); // the URL effect takes it
        setKind("authenticated");
        expect(win.close).not.toHaveBeenCalled();
        dispose();
    });

    it("releases when the start call settles with nothing waiting (it failed before starting)", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const { setKind, tracker, dispose } = setup();
        const settled = tracker.begin(URL_PROVIDER);
        setKind("failed");
        settled();
        expect(win.close).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("a new Connect closes the previous unused window, and the old one's ending doesn't touch the new", () => {
        const first = fakeWindow();
        const second = fakeWindow();
        reserve.mockReturnValueOnce(first).mockReturnValueOnce(second);
        const { setKind, tracker, dispose } = setup();
        const firstSettled = tracker.begin(URL_PROVIDER);
        setKind("failed");
        tracker.begin(URL_PROVIDER); // Retry
        expect(first.close).toHaveBeenCalledTimes(1);
        firstSettled();
        expect(second.close).not.toHaveBeenCalled();
        dispose();
    });

    it("releases on cleanup (the panel unmounts mid-connect)", () => {
        const win = fakeWindow();
        reserve.mockReturnValue(win);
        const { setKind, tracker, dispose } = setup();
        tracker.begin(URL_PROVIDER);
        setKind("waiting");
        dispose();
        expect(win.close).toHaveBeenCalledTimes(1);
    });
});
