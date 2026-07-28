// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { SETTLE_GRACE_MS, nextDoneCompletedAt, shouldNotifyOnReopen } from "./settled-grace";

describe("nextDoneCompletedAt", () => {
    it("starts the clock on first entering Done.completed", () => {
        expect(nextDoneCompletedAt("Done", "completed", null, 1000)).toBe(1000);
    });

    it("does not reset the clock on a re-render of the same Done.completed phase", () => {
        expect(nextDoneCompletedAt("Done", "completed", 1000, 2000)).toBe(1000);
    });

    it("clears the clock on Streaming", () => {
        expect(nextDoneCompletedAt("Streaming", undefined, 1000, 2000)).toBeNull();
    });

    it("clears the clock on other Done outcomes (stopped/errored/interrupted)", () => {
        expect(nextDoneCompletedAt("Done", "stopped", 1000, 2000)).toBeNull();
        expect(nextDoneCompletedAt("Done", "errored", 1000, 2000)).toBeNull();
    });

    it("clears the clock on Idle/Submitting/Disconnected", () => {
        expect(nextDoneCompletedAt("Idle", undefined, 1000, 2000)).toBeNull();
        expect(nextDoneCompletedAt("Submitting", undefined, 1000, 2000)).toBeNull();
        expect(nextDoneCompletedAt("Disconnected", undefined, 1000, 2000)).toBeNull();
    });
});

describe("shouldNotifyOnReopen", () => {
    it("is false when the pane was never in Done.completed (normal turn start)", () => {
        expect(shouldNotifyOnReopen(null, 10_000, SETTLE_GRACE_MS)).toBe(false);
    });

    it("is false for a re-promotion within the grace window (genuine same-breath continuation)", () => {
        const doneAt = 1000;
        const now = doneAt + SETTLE_GRACE_MS - 1;
        expect(shouldNotifyOnReopen(doneAt, now, SETTLE_GRACE_MS)).toBe(false);
    });

    it("is true once the grace window has fully elapsed", () => {
        const doneAt = 1000;
        const now = doneAt + SETTLE_GRACE_MS;
        expect(shouldNotifyOnReopen(doneAt, now, SETTLE_GRACE_MS)).toBe(true);
    });

    it("is true well after the grace window", () => {
        const doneAt = 1000;
        const now = doneAt + SETTLE_GRACE_MS * 10;
        expect(shouldNotifyOnReopen(doneAt, now, SETTLE_GRACE_MS)).toBe(true);
    });

    it("respects a custom grace window", () => {
        expect(shouldNotifyOnReopen(0, 100, 200)).toBe(false);
        expect(shouldNotifyOnReopen(0, 200, 200)).toBe(true);
    });
});
