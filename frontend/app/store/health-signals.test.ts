// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { applyHealthEvent, applySnapshot, worstLevel, type HealthKind, type HealthSignal } from "./health-signals";

const kinds = (s: HealthSignal[]) => s.map((x) => `${x.kind}:${x.level}`);

describe("applyHealthEvent", () => {
    it("adds a signal, and removes it when its kind returns to normal", () => {
        const one = applyHealthEvent([], { kind: "ram", level: "warn" }, 100);
        expect(kinds(one)).toEqual(["ram:warn"]);
        expect(one[0].since).toBe(100);
        expect(applyHealthEvent(one, { kind: "ram", level: "normal" }, 200)).toEqual([]);
    });

    it("keeps the episode's start when the level changes, and prefers the host's", () => {
        const warn = applyHealthEvent([], { kind: "backend", level: "warn", avg_ms: 1200 }, 100);
        const critical = applyHealthEvent(warn, { kind: "backend", level: "critical", avg_ms: 2900 }, 500);
        expect(critical[0].since).toBe(100);
        expect(critical[0].payload.avg_ms).toBe(2900);
        const stamped = applyHealthEvent([], { kind: "backend", level: "warn", since_ms: 42 }, 500);
        expect(stamped[0].since).toBe(42);
    });

    it("orders worst first, then backend, page file, RAM", () => {
        let s: HealthSignal[] = [];
        s = applyHealthEvent(s, { kind: "ram", level: "warn" }, 1);
        s = applyHealthEvent(s, { kind: "backend", level: "warn" }, 2);
        s = applyHealthEvent(s, { kind: "pagefile", level: "critical" }, 3);
        expect(kinds(s)).toEqual(["pagefile:critical", "backend:warn", "ram:warn"]);
        expect(worstLevel(s)).toBe("critical");
        expect(worstLevel([])).toBe("normal");
    });

    it("ignores unknown kinds and a normal for a kind that wasn't active", () => {
        const s = applyHealthEvent([], { kind: "ram", level: "warn" }, 1);
        expect(applyHealthEvent(s, { kind: "gpu" as HealthKind, level: "warn" }, 2)).toBe(s);
        expect(applyHealthEvent(s, { kind: "backend", level: "normal" }, 2)).toBe(s);
    });
});

describe("applySnapshot", () => {
    it("fills in what was active before the window opened", () => {
        const s = applySnapshot([], [{ kind: "pagefile", level: "warn", since_ms: 7 }], new Set(), 100);
        expect(kinds(s)).toEqual(["pagefile:warn"]);
        expect(s[0].since).toBe(7);
    });

    it("never overrides a kind an event has already reported", () => {
        // A recovery heard before the snapshot (read earlier) arrived.
        const s = applySnapshot([], [{ kind: "backend", level: "warn" }], new Set<HealthKind>(["backend"]), 100);
        expect(s).toEqual([]);
    });
});
