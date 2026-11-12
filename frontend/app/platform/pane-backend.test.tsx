// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({
    backend: "views" as string | undefined,
    clip: undefined as boolean | undefined,
    listen: vi.fn((_event: string, _handler: () => void) => Promise.resolve(() => {})),
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({ getAboutModalDetails: () => ({ paneBackend: env.backend, paneClip: env.clip }), listen: env.listen }),
}));

import { hidesPaneUnderOverlay, isViewsPanesOnWindows } from "./pane-backend";

const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36";

function at(userAgent: string, search: string): void {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
    window.history.replaceState(null, "", `/${search}`);
}

afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState(null, "", "/");
    env.backend = "views";
    env.clip = undefined;
});

describe("isViewsPanesOnWindows", () => {
    it("is the host's pane kind in an app window on Windows", () => {
        at(WINDOWS, "");
        expect(isViewsPanesOnWindows()).toBe(true);
        at(WINDOWS, "?windowLabel=window-pool-1");
        expect(isViewsPanesOnWindows()).toBe(true);
        env.backend = "hwnd";
        expect(isViewsPanesOnWindows()).toBe(false);
        env.backend = undefined;
        expect(isViewsPanesOnWindows()).toBe(false);
    });

    it("is never true in a floating pane's window, whose panes stay child windows", () => {
        at(WINDOWS, "?windowLabel=floating-1f0c");
        expect(isViewsPanesOnWindows()).toBe(false);
        at(WINDOWS, "?windowLabel=floating-pool-77aa");
        expect(isViewsPanesOnWindows()).toBe(false);
    });

    it("is never true off Windows", () => {
        at(LINUX, "");
        expect(isViewsPanesOnWindows()).toBe(false);
    });
});

describe("hidesPaneUnderOverlay", () => {
    it("is true for a Views pane on Linux or Windows whose host does not cut it", () => {
        at(LINUX, "");
        expect(hidesPaneUnderOverlay()).toBe(true);
        at(WINDOWS, "");
        expect(hidesPaneUnderOverlay()).toBe(true);
        env.clip = false;
        expect(hidesPaneUnderOverlay()).toBe(true);
    });

    it("is false once the host cuts the pane around the overlay", () => {
        env.clip = true;
        at(LINUX, "");
        expect(hidesPaneUnderOverlay()).toBe(false);
        at(WINDOWS, "");
        expect(hidesPaneUnderOverlay()).toBe(false);
    });

    it("is false for a Windows child-window pane and on macOS, which always get a hole", () => {
        env.backend = "hwnd";
        at(WINDOWS, "");
        expect(hidesPaneUnderOverlay()).toBe(false);
        env.backend = "views";
        at(WINDOWS, "?windowLabel=floating-1f0c");
        expect(hidesPaneUnderOverlay()).toBe(false);
        at(MAC, "");
        expect(hidesPaneUnderOverlay()).toBe(false);
    });
});

describe("a host whose runtime turns out to have no clip", () => {
    it("is heard once per window, and from then on the pane hides whole again", async () => {
        vi.resetModules();
        env.listen.mockClear();
        const fresh = await import("./pane-backend");
        env.clip = true;
        at(LINUX, "");
        fresh.watchPaneClip();
        fresh.watchPaneClip();
        expect(env.listen).toHaveBeenCalledTimes(1);
        expect(env.listen.mock.calls[0][0]).toBe("pane-clip-unavailable");
        expect(fresh.hidesPaneUnderOverlay()).toBe(false);

        env.listen.mock.calls[0][1]();
        expect(fresh.hidesPaneUnderOverlay()).toBe(true);
        at(WINDOWS, "");
        expect(fresh.hidesPaneUnderOverlay()).toBe(true);
        // A pane that gets a hole by other means is not affected.
        at(MAC, "");
        expect(fresh.hidesPaneUnderOverlay()).toBe(false);
    });
});
