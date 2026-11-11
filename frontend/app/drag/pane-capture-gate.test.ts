// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { beginPaneCapture, CAPTURE_GATE_MAX_MS, paneCaptureInFlight, resetPaneCaptureGateForTests } from "./pane-capture-gate";

beforeEach(() => {
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

    it("done twice is done once", () => {
        const done = beginPaneCapture();
        done();
        const next = beginPaneCapture();
        done();
        expect(paneCaptureInFlight()).toBe(true);
        next();
    });
});
