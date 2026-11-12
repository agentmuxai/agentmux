// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The browser host's AppApi: no host capability, desktop-only calls refused, and
// links opened only when they are web addresses.

import { NO_HOST_CAPS } from "@/util/host-caps";
import { afterEach, describe, expect, it, vi } from "vitest";
import { buildWebApi } from "./web-api";

afterEach(() => vi.restoreAllMocks());

describe("the browser host's AppApi", () => {
    it("reports every host capability off, so the UI hides what needs one", () => {
        const caps = buildWebApi().getHostCaps();
        expect(caps).toEqual(NO_HOST_CAPS);
        expect(Object.values(caps).every((v) => v === false)).toBe(true);
    });

    it("refuses what only a desktop host can do, naming it", async () => {
        const api = buildWebApi();
        await expect(api.openNewWindow()).rejects.toThrow("A new window isn't available in the browser");
        await expect(api.windows.getRect("main")).rejects.toThrow("windows.getRect isn't available in the browser");
        await expect(api.approvals.decideSsh("a", true, "", false)).rejects.toThrow(
            "approvals.decideSsh isn't available in the browser"
        );
        await expect(api.restartBackend()).rejects.toThrow("Restarting the backend isn't available in the browser");
    });

    it("leaves the optional host members out, so the UI's presence checks find no desktop host", () => {
        const { approvals, browserPanes } = buildWebApi();
        expect(typeof approvals.decideDesktop).toBe("undefined");
        expect(approvals.vault).toBeUndefined();
        expect(approvals.attestOperator).toBeUndefined();
        expect(approvals.appUpdate).toBeUndefined();
        expect(browserPanes.autocomplete).toBeUndefined();
    });

    it("holds no srv key: the proxy in front of srv adds it", () => {
        expect(buildWebApi().getAuthKey()).toBe("");
    });

    it("opens http and https addresses in a new tab without an opener", async () => {
        const open = vi.spyOn(window, "open").mockReturnValue(null);
        await buildWebApi().openExternalChecked("https://example.com/a?b=1");
        expect(open).toHaveBeenCalledWith("https://example.com/a?b=1", "_blank", "noopener,noreferrer");
    });

    it("refuses an address that is not a web address instead of opening it", async () => {
        const open = vi.spyOn(window, "open").mockReturnValue(null);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const api = buildWebApi();
        await expect(api.openExternalChecked("javascript:alert(1)")).rejects.toThrow("not a web address");
        api.openExternal("vscode://file/x");
        expect(open).not.toHaveBeenCalled();
        expect(warn).toHaveBeenCalled();
    });

    it("names the OS the browser runs on the way process.platform does", () => {
        const ua = vi.spyOn(window.navigator, "userAgent", "get");
        ua.mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
        expect(buildWebApi().getPlatform()).toBe("darwin");
        ua.mockReturnValue("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
        expect(buildWebApi().getPlatform()).toBe("win32");
        ua.mockReturnValue("Mozilla/5.0 (X11; Linux x86_64)");
        expect(buildWebApi().getPlatform()).toBe("linux");
    });
});
