// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { sleep } from "./async";

describe("sleep", () => {
    it("resolves after the given delay", async () => {
        vi.useFakeTimers();
        const spy = vi.fn();
        sleep(1000).then(spy);
        await vi.advanceTimersByTimeAsync(999);
        expect(spy).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(1);
        expect(spy).toHaveBeenCalledOnce();
        vi.useRealTimers();
    });

    it("resolves with undefined", async () => {
        vi.useFakeTimers();
        const p = sleep(0);
        await vi.advanceTimersByTimeAsync(0);
        expect(await p).toBeUndefined();
        vi.useRealTimers();
    });
});
