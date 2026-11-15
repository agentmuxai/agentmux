// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * srv's own panics reach the status bar's health store as the persisted `srv-panics` mux event (#2578): a `panics`
 * signal that turns the backend dot, and a `normal` event clears it.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mux = vi.hoisted(() => ({ handlers: [] as Array<(p: unknown) => void>, off: vi.fn() }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_type: string, handler: (p: unknown) => void) => {
        mux.handlers.push(handler);
        return mux.off;
    },
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        listen: async () => () => {},
        getHealthSignals: async () => [],
    }),
}));

import { applyHealthEvent, healthSignals, startHealthSignals, worstHealthLevel } from "./health-signals";

const panics = (level: "normal" | "warn" | "critical", extra = {}) => ({
    kind: "panics" as const,
    level,
    since_ms: level === "normal" ? null : 5000,
    last_minute: 30,
    total: 200,
    recovered_lock: 0,
    top: [{ location: "backend/reactive/handler.rs:1416", count: 30 }],
    ...extra,
});

describe("the panics signal", () => {
    it("is a kind of its own, after the slow backend and before disk", () => {
        let s = applyHealthEvent([], { kind: "disk", level: "warn" }, 1);
        s = applyHealthEvent(s, panics("warn"), 2);
        s = applyHealthEvent(s, { kind: "backend", level: "warn" }, 3);
        expect(s.map((x) => x.kind)).toEqual(["backend", "panics", "disk"]);
    });

    it("starts at the episode's start srv names, and a normal event clears it", () => {
        const on = applyHealthEvent([], panics("warn"), 100);
        expect(on[0].since).toBe(5000);
        expect(applyHealthEvent(on, panics("normal"), 200)).toEqual([]);
    });
});

describe("startHealthSignals follows srv-panics", () => {
    let stop: () => void;
    beforeEach(() => {
        mux.handlers.length = 0;
        mux.off.mockClear();
        stop = startHealthSignals();
    });
    afterEach(() => stop());

    it("turns the dot on a warn, to critical, and back to normal", () => {
        expect(mux.handlers).toHaveLength(1);
        expect(worstHealthLevel()).toBe("normal");
        mux.handlers[0](panics("warn"));
        expect(worstHealthLevel()).toBe("warn");
        expect(healthSignals()[0].payload.last_minute).toBe(30);
        mux.handlers[0](panics("critical", { last_minute: 400 }));
        expect(worstHealthLevel()).toBe("critical");
        expect(healthSignals()).toHaveLength(1);
        mux.handlers[0](panics("normal"));
        expect(healthSignals()).toEqual([]);
    });

    it("stops listening when stopped", () => {
        stop();
        expect(mux.off).toHaveBeenCalledTimes(1);
    });
});
