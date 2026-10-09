// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { statusDotColor, statusDotTip } from "./backend-dot";

describe("statusDotColor", () => {
    it("is yellow while srv runs with any signal active, warn or critical", () => {
        expect(statusDotColor("running", "normal")).toBe("var(--accent-color)");
        expect(statusDotColor("running", "warn")).toBe("var(--warning-color)");
        expect(statusDotColor("running", "critical")).toBe("var(--warning-color)");
    });

    it("keeps connecting yellow and crashed red whatever the signals say", () => {
        expect(statusDotColor("connecting", "normal")).toBe("var(--warning-color)");
        expect(statusDotColor("crashed", "critical")).toBe("var(--error-color)");
        expect(statusDotColor(null, "warn")).toBeNull();
    });
});

describe("statusDotTip", () => {
    it("names the active signals", () => {
        expect(statusDotTip([])).toBe("Backend status, click for details");
        expect(statusDotTip(["Low RAM", "Slow backend 1.8s"])).toBe(
            "Backend status: Low RAM, Slow backend 1.8s. Click for details"
        );
    });
});
