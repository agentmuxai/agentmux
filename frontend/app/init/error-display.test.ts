// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, beforeEach, vi } from "vitest";

const mocks = vi.hoisted(() => ({ writeText: vi.fn(() => Promise.resolve()) }));
vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText }));

import { failStartup, reloadKeepingHostCredentials, showStartupError, StartupFailureHandled } from "./error-display";

describe("showStartupError", () => {
    beforeEach(() => {
        document.body.innerHTML = '<div id="startup-loading">Loading...</div><div id="main"></div>';
        document.body.style.visibility = "hidden";
        document.body.style.opacity = "0";
        document.body.classList.add("is-transparent");
    });

    it("makes the body visible", () => {
        showStartupError("test error");
        expect(document.body.style.visibility).toBe("visible");
        expect(document.body.style.opacity).toBe("1");
        expect(document.body.classList.contains("is-transparent")).toBe(false);
    });

    it("shows the app root too: the startup guard hides #main, not the body", () => {
        const main = document.getElementById("main")!;
        main.style.visibility = "hidden";
        main.style.opacity = "0";
        showStartupError("test error");
        expect(main.style.visibility).toBe("visible");
        expect(main.style.opacity).toBe("1");
    });

    it("removes the startup loader", () => {
        showStartupError("test error");
        expect(document.getElementById("startup-loading")).toBeNull();
    });

    it("shows the error message in #main", () => {
        showStartupError("something broke");
        const main = document.getElementById("main");
        // The raw error lives in the collapsible technical-details <pre>.
        expect(main?.textContent).toContain("something broke");
    });

    it("renders Restore and Copy details — the two old (now-retired) buttons stay gone", () => {
        showStartupError("test error");
        const main = document.getElementById("main");
        const buttons = main?.querySelectorAll("button") ?? [];
        // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 4 added "Copy
        // details" alongside Restore — the assertion this test's name still
        // references (down to exactly one button) predates that spec.
        expect(buttons.length).toBe(2);
        expect(buttons[0]?.textContent).toContain("Restore");
        expect(buttons[1]?.textContent).toContain("Copy details");
        // The retired labels must not reappear.
        expect(main?.textContent).not.toContain("Reopen window");
    });

    it("handles missing #main gracefully", () => {
        document.body.innerHTML = "";
        expect(() => showStartupError("no main div")).not.toThrow();
    });

    it("Copy details uses the DOM transport (bridge may be down), copying a redacted report, and stays reachable while collapsed", async () => {
        let capturedText: string | undefined;
        const execSpy = vi.fn(() => {
            capturedText = (document.activeElement as HTMLTextAreaElement | null)?.value;
            return true;
        });
        document.execCommand = execSpy;

        showStartupError("TypeError: Failed to fetch\nAuthorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz0123");
        const main = document.getElementById("main")!;

        // §4.4 rule 2: the copy button is not nested inside <details>, so it
        // stays visible/clickable whether or not "Technical details" is expanded.
        const details = main.querySelector("details");
        expect(details?.hasAttribute("open")).toBe(false);

        const copyBtn = Array.from(main.querySelectorAll("button")).find((b) => b.textContent?.includes("Copy details"))!;
        copyBtn.click();
        await Promise.resolve();
        await Promise.resolve();

        expect(mocks.writeText).not.toHaveBeenCalled(); // IPC skipped — transport="dom"
        expect(execSpy).toHaveBeenCalledWith("copy");
        expect(capturedText).toContain("Failed to fetch");
        expect(capturedText).not.toContain("ghp_abcdef");
        expect(capturedText).toContain("[redacted");

        delete document.execCommand;
    });
});

/**
 * #3868: initHostMux used to put up the card and return normally, so bootstrap
 * logged "✅ Main application loaded successfully", reset the reload budget,
 * and one refused request left the window dead. failStartup routes a startup
 * failure through the same bounded auto-reload as a bootstrap failure and
 * always throws, so nothing upstream mistakes it for success.
 */
describe("failStartup", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        sessionStorage.clear();
        document.body.innerHTML = '<div id="startup-loading">Loading...</div><div id="main"></div>';
    });
    afterEach(() => {
        vi.useRealTimers();
        sessionStorage.clear();
    });

    it("throws StartupFailureHandled carrying the message", () => {
        expect(() => failStartup("startup: client \"c1\" did not load")).toThrow(StartupFailureHandled);
        try {
            failStartup("boom");
        } catch (e) {
            expect((e as Error).message).toBe("boom");
        }
    });

    it("schedules an auto-reload while the budget lasts, instead of the card", () => {
        expect(() => failStartup("boom")).toThrow(StartupFailureHandled);
        expect(document.getElementById("main")?.textContent).toContain("Reconnecting");
    });

    it("shows the card once the reload budget is spent", () => {
        for (let i = 0; i < 3; i++) expect(() => failStartup("boom")).toThrow(StartupFailureHandled);
        expect(() => failStartup("boom")).toThrow(StartupFailureHandled);
        const text = document.getElementById("main")?.textContent ?? "";
        expect(text).toContain("AgentMux couldn't start this window");
        expect(text).not.toContain("Reconnecting");
    });
});

describe("showStartupError copy", () => {
    beforeEach(() => {
        sessionStorage.clear();
        document.body.innerHTML = '<div id="main"></div>';
    });

    it("does not claim the host connection was lost — the card also covers a failed request", () => {
        showStartupError("startup: tab \"t1\" did not load");
        const text = document.getElementById("main")?.textContent ?? "";
        expect(text).toContain("This window didn't finish starting");
        expect(text).not.toContain("lost its connection");
    });
});

describe("reloadKeepingHostCredentials", () => {
    const nav = { href: "http://127.0.0.1:1234/?windowLabel=main", assign: vi.fn(), replace: vi.fn(), reload: vi.fn() };
    beforeEach(() => {
        vi.stubGlobal("location", nav);
        nav.assign.mockClear();
        nav.replace.mockClear();
        nav.reload.mockClear();
    });
    afterEach(() => {
        vi.unstubAllGlobals();
        delete window.__AGENTMUX_IPC_PORT__;
        delete window.__AGENTMUX_IPC_TOKEN__;
    });

    // A navigation that adds a history entry leaves the old page in the
    // back/forward cache, so the reload would free nothing.
    it("replaces the history entry with the credentialed URL", () => {
        window.__AGENTMUX_IPC_PORT__ = 1234;
        window.__AGENTMUX_IPC_TOKEN__ = "t";
        reloadKeepingHostCredentials();
        expect(nav.assign).not.toHaveBeenCalled();
        expect(nav.replace).toHaveBeenCalledTimes(1);
        const url = new URL(nav.replace.mock.calls[0][0]);
        expect(url.searchParams.get("ipc_port")).toBe("1234");
        expect(url.searchParams.get("ipc_token")).toBe("t");
        expect(url.searchParams.get("windowLabel")).toBe("main");
    });

    it("falls back to a plain reload without creds", () => {
        reloadKeepingHostCredentials();
        expect(nav.reload).toHaveBeenCalledTimes(1);
        expect(nav.assign).not.toHaveBeenCalled();
        expect(nav.replace).not.toHaveBeenCalled();
    });
});
