// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A loopback request can fail without ever reaching srv — on Windows a
 * WSAENOBUFS (10055) connect refusal surfaces in the page as
 * `TypeError: Failed to fetch`. One such failure during startup used to fail
 * the whole window (#3868). These pin which errors count as transient and
 * that idempotent reads retry them, and only them.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { isTransientNetworkError, retryTransient } from "./transient-network";

describe("isTransientNetworkError", () => {
    it("treats a fetch that got no response as transient", () => {
        expect(isTransientNetworkError(new TypeError("Failed to fetch"))).toBe(true);
        expect(isTransientNetworkError(new TypeError("NetworkError when attempting to fetch resource."))).toBe(true);
    });

    it("does not treat a backend answer or a code bug as transient", () => {
        expect(isTransientNetworkError(new Error("call object.GetObject error: not found: tab:x"))).toBe(false);
        expect(isTransientNetworkError(new Error("call object.GetObject failed: 500 Internal Server Error"))).toBe(
            false
        );
        expect(isTransientNetworkError(new TypeError("Cannot read properties of null (reading 'oid')"))).toBe(false);
        expect(isTransientNetworkError("Failed to fetch")).toBe(false);
    });
});

describe("retryTransient", () => {
    afterEach(() => {
        vi.useRealTimers();
    });

    it("returns the first success after transient failures", async () => {
        const fn = vi
            .fn()
            .mockRejectedValueOnce(new TypeError("Failed to fetch"))
            .mockRejectedValueOnce(new TypeError("Failed to fetch"))
            .mockResolvedValueOnce("ok");
        await expect(retryTransient(fn, [0, 0, 0])).resolves.toBe("ok");
        expect(fn).toHaveBeenCalledTimes(3);
    });

    it("gives up after the last delay and rethrows the network error", async () => {
        const fn = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
        await expect(retryTransient(fn, [0, 0])).rejects.toThrow("Failed to fetch");
        expect(fn).toHaveBeenCalledTimes(3);
    });

    it("does not retry a non-transient error", async () => {
        const fn = vi.fn().mockRejectedValue(new Error("call object.GetObject error: not found: tab:x"));
        await expect(retryTransient(fn, [0, 0, 0])).rejects.toThrow("not found");
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("waits the given delays between attempts", async () => {
        vi.useFakeTimers();
        const fn = vi.fn().mockRejectedValueOnce(new TypeError("Failed to fetch")).mockResolvedValueOnce("ok");
        const p = retryTransient(fn, [250]);
        await vi.advanceTimersByTimeAsync(249);
        expect(fn).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(1);
        await expect(p).resolves.toBe("ok");
        expect(fn).toHaveBeenCalledTimes(2);
    });
});
