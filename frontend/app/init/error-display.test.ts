// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, beforeEach, vi } from "vitest";

const mocks = vi.hoisted(() => ({ writeText: vi.fn(() => Promise.resolve()) }));
vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText }));

import { showStartupError } from "./error-display";

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
