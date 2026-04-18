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
        expect(main?.textContent).toContain("AgentMux failed to start");
        expect(main?.textContent).toContain("something broke");
    });

    it("handles missing #main gracefully", () => {
        document.body.innerHTML = "";
        expect(() => showStartupError("no main div")).not.toThrow();
    });
});
