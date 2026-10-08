// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    backendMessage,
    formatAvg,
    formatSince,
    healthLabel,
    messageFor,
    pagefileGuidance,
} from "./health-signals-text";

describe("healthLabel", () => {
    it("names each kind, with the backend's average when known", () => {
        expect(healthLabel("ram")).toBe("Low RAM");
        expect(healthLabel("pagefile")).toBe("Low page file");
        expect(healthLabel("backend", { kind: "backend", level: "warn", avg_ms: 1840 })).toBe("Slow backend 1.8s");
        expect(healthLabel("backend", { kind: "backend", level: "warn" })).toBe("Slow backend");
    });
});

describe("formatSince", () => {
    it("reads as a duration", () => {
        expect(formatSince(20_000)).toBe("just now");
        expect(formatSince(4 * 60_000 + 5_000)).toBe("for 4 min");
        expect(formatSince(60 * 60_000)).toBe("for 1 h");
        expect(formatSince(95 * 60_000)).toBe("for 1 h 35 min");
        expect(formatSince(-5)).toBe("just now");
    });
});

describe("pagefileGuidance", () => {
    it("warns about a fixed-size page file regardless of free disk", () => {
        expect(pagefileGuidance(false, 90)).toMatch(/fixed size/);
        expect(pagefileGuidance(false, undefined)).toMatch(/fixed size/);
    });

    it("warns Windows can't grow the page file when disk is low and system-managed", () => {
        expect(pagefileGuidance(true, 5)).toMatch(/can't grow/);
        expect(pagefileGuidance(true, 19.9)).toMatch(/can't grow/);
    });

    it("uses the soft framing when system-managed with healthy or unknown free disk", () => {
        expect(pagefileGuidance(true, 20)).toMatch(/expand virtual memory automatically/);
        expect(pagefileGuidance(true, undefined)).toMatch(/expand virtual memory automatically/);
    });

    it("returns no guidance when system-managed status is unknown (fail-open, no guess)", () => {
        expect(pagefileGuidance(undefined, undefined)).toBe("");
        expect(pagefileGuidance(undefined, 5)).toBe("");
    });
});

describe("messageFor", () => {
    it("RAM messages never include page-file/disk guidance", () => {
        const msg = messageFor("ram", "critical", { kind: "ram", level: "critical" });
        expect(msg).toMatch(/RAM/);
        expect(msg).not.toMatch(/page file|disk/i);
    });

    it("pagefile messages append disk-aware guidance", () => {
        const msg = messageFor("pagefile", "critical", { kind: "pagefile", level: "critical", system_managed: false });
        expect(msg).toMatch(/page file/i);
        expect(msg).toMatch(/fixed size/);
    });

    it("backend: warn names the likely cause, critical that it may restart, never memory", () => {
        expect(formatAvg(1840)).toBe("1.8s");
        expect(formatAvg(undefined)).toBe("");
        expect(formatAvg(Number.NaN)).toBe("");
        const warn = messageFor("backend", "warn", { kind: "backend", level: "warn", avg_ms: 1200 });
        expect(warn).toBe(backendMessage("warn", 1200));
        expect(warn).toMatch(/responding slowly \(avg 1\.2s\)/);
        expect(warn).toMatch(/agents' builds/);
        const critical = messageFor("backend", "critical", { kind: "backend", level: "critical", avg_ms: 2900 });
        expect(critical).toMatch(/barely responding \(avg 2\.9s\)/);
        expect(critical).toMatch(/restart itself/);
        for (const level of ["warn", "critical"] as const) {
            expect(messageFor("backend", level, { kind: "backend", level })).not.toMatch(/page file|RAM|memory/i);
        }
    });
});
