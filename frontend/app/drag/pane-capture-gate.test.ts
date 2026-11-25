// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const logged = vi.hoisted(() => ({ lines: [] as Array<{ msg: string; data: Record<string, unknown> }> }));
vi.mock("@/util/logger", () => ({
    Logger: { info: (_m: string, msg: string, data: Record<string, unknown>) => logged.lines.push({ msg, data }) },
}));
import { beginPaneCapture, CAPTURE_GATE_MAX_MS, paneCaptureInFlight, resetPaneCaptureGateForTests } from "./pane-capture-gate";

beforeEach(() => {
    logged.lines = [];
    vi.useFakeTimers();
    resetPaneCaptureGateForTests();
});
afterEach(() => vi.useRealTimers());

describe("the gate a pane's picture holds the drag's drawing behind", () => {
    it("is closed from the capture's start until it is done", () => {
        expect(paneCaptureInFlight()).toBe(false);
        const done = beginPaneCapture();
        expect(paneCaptureInFlight()).toBe(true);
        done();
        expect(paneCaptureInFlight()).toBe(false);
    });

    it("opens by itself when a capture stalls", () => {
        beginPaneCapture();
        vi.advanceTimersByTime(CAPTURE_GATE_MAX_MS - 1);
        expect(paneCaptureInFlight()).toBe(true);
        vi.advanceTimersByTime(1);
        expect(paneCaptureInFlight()).toBe(false);
    });

    it("follows the newest capture: an older one finishing, or timing out, does not open it", () => {
        const first = beginPaneCapture();
        vi.advanceTimersByTime(CAPTURE_GATE_MAX_MS - 50);
        const second = beginPaneCapture();
        first();
        expect(paneCaptureInFlight()).toBe(true);
        // The first capture's own timer runs out.
        vi.advanceTimersByTime(60);
        expect(paneCaptureInFlight()).toBe(true);
        second();
        expect(paneCaptureInFlight()).toBe(false);
    });

    it("logs how long the drag's drawing was held when the drag began during the capture", () => {
        const done = beginPaneCapture();
        vi.advanceTimersByTime(30);
        document.dispatchEvent(new Event("dragstart"));
        vi.advanceTimersByTime(85);
        done();
        expect(logged.lines).toHaveLength(1);
        expect(logged.lines[0].msg).toContain("[tearoff-perf]");
        expect(logged.lines[0].data).toEqual({ heldMs: 85, timedOut: false });
    });

    it("logs a drag held to the gate's limit as timed out", () => {
        beginPaneCapture();
        document.dispatchEvent(new Event("dragstart"));
        vi.advanceTimersByTime(CAPTURE_GATE_MAX_MS);
        expect(logged.lines[0].data).toEqual({ heldMs: CAPTURE_GATE_MAX_MS, timedOut: true });
    });

    it("logs no hold when the picture was ready before the drag began, and nothing when there was no picture", () => {
        document.dispatchEvent(new Event("dragstart"));
        expect(logged.lines).toEqual([]);
        const done = beginPaneCapture();
        done();
        vi.advanceTimersByTime(40);
        document.dispatchEvent(new Event("dragstart"));
        expect(logged.lines).toHaveLength(1);
        expect(logged.lines[0].data).toEqual({ heldMs: 0, readyBeforeMs: 40 });
    });

    it("done twice is done once", () => {
        const done = beginPaneCapture();
        done();
        const next = beginPaneCapture();
        done();
        expect(paneCaptureInFlight()).toBe(true);
        next();
    });
});
