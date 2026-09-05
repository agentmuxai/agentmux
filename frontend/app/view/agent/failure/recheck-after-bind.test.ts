// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import { retryRecheckAfterBind } from "./recheck-after-bind";

const noopSleep = async () => {};

describe("retryRecheckAfterBind", () => {
    it("succeeds immediately if the first recheck is already true (no sleep needed)", async () => {
        const recheck = vi.fn().mockResolvedValue(true);
        const onHealthy = vi.fn();
        const sleep = vi.fn(noopSleep);

        const result = await retryRecheckAfterBind({
            recheck,
            stillBlocked: () => true,
            sleep,
            onHealthy,
        });

        expect(result).toBe(true);
        expect(recheck).toHaveBeenCalledTimes(1);
        expect(onHealthy).toHaveBeenCalledTimes(1);
        expect(sleep).not.toHaveBeenCalled();
    });

    it("retries through failures and picks up a later success — the actual race this fixes", async () => {
        // Simulates the real bug: the first recheck races bindAccountToAgent's
        // cmd:env refresh and reads stale env; a later attempt (after the
        // refresh has landed) succeeds.
        const recheck = vi.fn()
            .mockResolvedValueOnce(false)
            .mockResolvedValueOnce(false)
            .mockResolvedValueOnce(true);
        const onHealthy = vi.fn();

        const result = await retryRecheckAfterBind(
            { recheck, stillBlocked: () => true, sleep: noopSleep, onHealthy },
            [10, 10, 10],
        );

        expect(result).toBe(true);
        expect(recheck).toHaveBeenCalledTimes(3);
        expect(onHealthy).toHaveBeenCalledTimes(1);
    });

    it("stops calling onHealthy after the first success — never fires twice", async () => {
        const recheck = vi.fn().mockResolvedValue(true);
        const onHealthy = vi.fn();
        await retryRecheckAfterBind({ recheck, stillBlocked: () => true, sleep: noopSleep, onHealthy });
        expect(onHealthy).toHaveBeenCalledTimes(1);
    });

    it("gives up after exhausting the ladder without ever succeeding", async () => {
        const recheck = vi.fn().mockResolvedValue(false);
        const onHealthy = vi.fn();

        const result = await retryRecheckAfterBind(
            { recheck, stillBlocked: () => true, sleep: noopSleep, onHealthy },
            [10, 10],
        );

        expect(result).toBe(false);
        // 2 delays → 3 total attempts (delays.length + 1 — see the function's
        // own doc comment on why the final attempt is explicit).
        expect(recheck).toHaveBeenCalledTimes(3);
        expect(onHealthy).not.toHaveBeenCalled();
    });

    it("bails early once stillBlocked() turns false, without exhausting the ladder", async () => {
        let blocked = true;
        const recheck = vi.fn().mockResolvedValue(false);
        const onHealthy = vi.fn();

        const result = await retryRecheckAfterBind(
            { recheck, stillBlocked: () => blocked, sleep: async () => { blocked = false; }, onHealthy },
            [10, 10, 10],
        );

        expect(result).toBe(false);
        // One attempt, one sleep (which flips stillBlocked to false), then bail
        // — must NOT continue through the remaining two delays.
        expect(recheck).toHaveBeenCalledTimes(1);
        expect(onHealthy).not.toHaveBeenCalled();
    });

    it("sleeps the exact delay ladder values, in order, between attempts", async () => {
        const recheck = vi.fn().mockResolvedValue(false);
        const sleepCalls: number[] = [];
        const sleep = async (ms: number) => { sleepCalls.push(ms); };

        await retryRecheckAfterBind(
            { recheck, stillBlocked: () => true, sleep, onHealthy: () => {} },
            [300, 700, 1500],
        );

        expect(sleepCalls).toEqual([300, 700, 1500]);
    });
});
