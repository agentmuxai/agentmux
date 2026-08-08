// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    messageFor,
    pagefileGuidance,
    severity,
    shouldShow,
    type PressureLevel,
} from "./memory-pressure-banner";

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

describe("memory-pressure banner — pagefileGuidance", () => {
    it("warns about a fixed-size page file regardless of free disk", () => {
        expect(pagefileGuidance(false, 90)).toMatch(/fixed size/);
        expect(pagefileGuidance(false, undefined)).toMatch(/fixed size/);
    });

    it("warns Windows can't grow the page file when disk is low and system-managed", () => {
        expect(pagefileGuidance(true, 5)).toMatch(/can't grow/);
        expect(pagefileGuidance(true, 19.9)).toMatch(/can't grow/);
    });

    it("uses the soft framing when system-managed with healthy free disk", () => {
        expect(pagefileGuidance(true, 20)).toMatch(/expand virtual memory automatically/);
        expect(pagefileGuidance(true, 80)).toMatch(/expand virtual memory automatically/);
    });

    it("returns no guidance when system-managed status is unknown (fail-open, no guess)", () => {
        expect(pagefileGuidance(undefined, undefined)).toBe("");
        expect(pagefileGuidance(undefined, 5)).toBe("");
    });

    it("falls back to the soft framing when managed is known but disk is unknown", () => {
        // system_managed known + disk_free_pct missing -> can't tell if it's
        // stuck, so default to the less alarming framing rather than silence.
        expect(pagefileGuidance(true, undefined)).toMatch(/expand virtual memory automatically/);
    });
});

describe("memory-pressure banner — messageFor", () => {
    it("RAM messages never include page-file/disk guidance", () => {
        const msg = messageFor("ram", "critical", { kind: "ram", level: "critical" });
        expect(msg).toMatch(/RAM/);
        expect(msg).not.toMatch(/page file|disk/i);
    });

    it("pagefile messages append disk-aware guidance", () => {
        const msg = messageFor("pagefile", "critical", {
            kind: "pagefile",
            level: "critical",
            system_managed: false,
        });
        expect(msg).toMatch(/page file/i);
        expect(msg).toMatch(/fixed size/);
    });
});
