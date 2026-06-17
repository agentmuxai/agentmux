// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * usePtyWidth — cols math + the initial-resize readiness gate.
 *
 * The gate is the heart of the PTY-resize race fix
 * (docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md): a resize must NOT
 * be sent before the controller can accept input, or it fails with "controller
 * is not running" (the "failed after 3 attempts" warning). So sends are
 * deferred until the controller is "running" (or already running on re-mount),
 * coalescing the latest width; transient failures retry, permanent ones don't.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    // Set per-test so each can control the resize RPC + status probe.
    rpc: undefined as unknown as ReturnType<typeof vi.fn>,
    getStatus: undefined as unknown as ReturnType<typeof vi.fn>,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ControllerInputCommand: (...args: unknown[]) => hub.rpc(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    BlockService: { GetControllerStatus: (...args: unknown[]) => hub.getStatus(...args) },
}));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));
vi.mock("@/app/store/wos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));

import { usePtyWidth, __test__ } from "./usePtyWidth";

const { DEBOUNCE_MS } = __test__;

// ── Pure helpers ─────────────────────────────────────────────────────────

describe("computeCols", () => {
    it("converts width→cols using the padding + monospace ratio", () => {
        // cell = 15 * 0.6 = 9; usable = 800 - 16 = 784; floor(784 / 9) = 87.
        expect(__test__.computeCols(800, 15)).toBe(87);
    });
    it("floors at MIN_COLS for a very narrow pane", () => {
        expect(__test__.computeCols(50, 15)).toBe(__test__.MIN_COLS);
    });
});

describe("computeTermSizeFromEl", () => {
    it("returns rows=25 + cols≥MIN for a laid-out element", () => {
        const el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { value: 800, configurable: true });
        const ts = __test__.computeTermSizeFromEl(el);
        expect(ts?.rows).toBe(25);
        expect(ts?.cols).toBeGreaterThanOrEqual(__test__.MIN_COLS);
    });
    it("returns undefined when absent or not yet laid out", () => {
        expect(__test__.computeTermSizeFromEl(undefined)).toBeUndefined();
        const el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { value: 0, configurable: true });
        expect(__test__.computeTermSizeFromEl(el)).toBeUndefined();
    });
});

describe("isRetryableResizeError", () => {
    it("treats controller-not-ready errors as transient", () => {
        expect(__test__.isRetryableResizeError("controller is not running")).toBe(true);
        expect(__test__.isRetryableResizeError("no controller for block abc")).toBe(true);
    });
    it("treats other errors as permanent", () => {
        expect(__test__.isRetryableResizeError("malformed termsize")).toBe(false);
        expect(__test__.isRetryableResizeError("")).toBe(false);
    });
});

// ── Readiness gate + retry (integration) ───────────────────────────────────

describe("usePtyWidth readiness gate", () => {
    let el: HTMLDivElement;
    let roCb: (() => void) | undefined;

    // Flush a few microtask ticks so onMount + the GetControllerStatus probe
    // (a resolved-promise .then chain) settle.
    const settle = async () => {
        for (let i = 0; i < 4; i++) await Promise.resolve();
    };
    const fire = (type: string, data: unknown) => {
        const h = hub.handlers.get(type);
        if (!h) throw new Error(`no "${type}" handler — usePtyWidth onMount did not run`);
        h({ data });
    };
    const mount = () => usePtyWidth({ blockId: "b", elementRef: () => el, log: () => {} });

    beforeEach(() => {
        hub.handlers.clear();
        hub.rpc = vi.fn().mockResolvedValue(undefined);
        hub.getStatus = vi.fn().mockResolvedValue({ shellprocstatus: "init" });
        vi.useFakeTimers();
        el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { value: 800, configurable: true });
        roCb = undefined;
        // Stub ResizeObserver: capture the callback so a resize can be simulated.
        (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
            constructor(cb: () => void) {
                roCb = cb;
            }
            observe() {}
            unobserve() {}
            disconnect() {}
        };
    });
    afterEach(() => vi.useRealTimers());

    it("defers the initial resize until the controller reports 'running'", async () => {
        await createRoot(async (dispose) => {
            mount();
            await settle(); // onMount + probe (status: init → not ready)
            roCb?.(); // at-mount ResizeObserver delivery
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
            expect(hub.rpc).not.toHaveBeenCalled(); // deferred — not running yet

            fire("controllerstatus", { shellprocstatus: "running" });
            expect(hub.rpc).toHaveBeenCalledTimes(1); // flushed on running
            dispose();
        });
    });

    it("sends immediately when the controller is already running (re-mount)", async () => {
        hub.getStatus = vi.fn().mockResolvedValue({ shellprocstatus: "running" });
        await createRoot(async (dispose) => {
            mount();
            await settle(); // probe → already running → ready
            roCb?.();
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
            expect(hub.rpc).toHaveBeenCalledTimes(1);
            dispose();
        });
    });

    it("retries a transient 'controller is not running' failure, then succeeds", async () => {
        hub.getStatus = vi.fn().mockResolvedValue({ shellprocstatus: "running" });
        hub.rpc = vi
            .fn()
            .mockRejectedValueOnce(new Error("controller is not running"))
            .mockResolvedValueOnce(undefined);
        await createRoot(async (dispose) => {
            mount();
            await settle();
            roCb?.();
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS); // attempt 1 → rejects
            expect(hub.rpc).toHaveBeenCalledTimes(1);
            await vi.advanceTimersByTimeAsync(700); // jittered backoff (≤600ms) → retry
            expect(hub.rpc).toHaveBeenCalledTimes(2);
            dispose();
        });
    });

    it("does not retry a permanent failure", async () => {
        hub.getStatus = vi.fn().mockResolvedValue({ shellprocstatus: "running" });
        hub.rpc = vi.fn().mockRejectedValue(new Error("some permanent error"));
        await createRoot(async (dispose) => {
            mount();
            await settle();
            roCb?.();
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS); // attempt 1 → rejects (permanent)
            expect(hub.rpc).toHaveBeenCalledTimes(1);
            await vi.advanceTimersByTimeAsync(2000);
            expect(hub.rpc).toHaveBeenCalledTimes(1); // no retry
            dispose();
        });
    });
});
