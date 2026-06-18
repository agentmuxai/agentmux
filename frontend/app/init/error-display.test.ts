// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, beforeEach } from "vitest";
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

    it("renders a single Restore button (the two old buttons are consolidated)", () => {
        showStartupError("test error");
        const main = document.getElementById("main");
        const buttons = main?.querySelectorAll("button") ?? [];
        expect(buttons.length).toBe(1);
        expect(buttons[0]?.textContent).toContain("Restore");
        // The retired labels must not reappear.
        expect(main?.textContent).not.toContain("Reopen window");
    });

    it("handles missing #main gracefully", () => {
        document.body.innerHTML = "";
        expect(() => showStartupError("no main div")).not.toThrow();
    });
});
