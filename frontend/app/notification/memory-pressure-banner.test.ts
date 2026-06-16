// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { severity, shouldShow, type PressureLevel } from "./memory-pressure-banner";

describe("memory-pressure banner — severity", () => {
    it("orders normal < warn < critical", () => {
        expect(severity("normal")).toBe(0);
        expect(severity("warn")).toBe(1);
        expect(severity("critical")).toBe(2);
    });
});

describe("memory-pressure banner — shouldShow", () => {
    const notDismissed: PressureLevel = "normal";

    it("hides at normal regardless of dismissal", () => {
        expect(shouldShow("normal", "normal")).toBe(false);
        expect(shouldShow("normal", "warn")).toBe(false);
    });

    it("shows on warn/critical when not dismissed", () => {
        expect(shouldShow("warn", notDismissed)).toBe(true);
        expect(shouldShow("critical", notDismissed)).toBe(true);
    });

    it("stays hidden after dismissing at the same level", () => {
        expect(shouldShow("warn", "warn")).toBe(false);
        expect(shouldShow("critical", "critical")).toBe(false);
    });

    it("re-shows when pressure escalates past the dismissed level", () => {
        // Dismissed at warn, then escalates to critical → show again.
        expect(shouldShow("critical", "warn")).toBe(true);
    });

    it("stays hidden when pressure de-escalates below the dismissed level", () => {
        // Dismissed at critical, drops to warn → still hidden (less severe).
        expect(shouldShow("warn", "critical")).toBe(false);
    });
});
