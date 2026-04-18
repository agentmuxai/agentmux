// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { withTimeout } from "./timeout";

describe("withTimeout", () => {
    it("resolves when promise completes before timeout", async () => {
        const result = await withTimeout(
            Promise.resolve("ok"),
            1000,
            "test"
        );
        expect(result).toBe("ok");
    });

    it("rejects with timeout error when promise is too slow", async () => {
        const slowPromise = new Promise<string>((resolve) =>
            setTimeout(() => resolve("late"), 5000)
        );
        await expect(
            withTimeout(slowPromise, 50, "slow-op")
        ).rejects.toThrow("Timeout: slow-op did not respond within 0.05s");
    });

    it("preserves the original rejection if it happens before timeout", async () => {
        const failingPromise = Promise.reject(new Error("original error"));
        await expect(
            withTimeout(failingPromise, 1000, "test")
        ).rejects.toThrow("original error");
    });
});
