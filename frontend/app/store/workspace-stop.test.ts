// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A hosted workspace's pending stop as the window keeps it: what it shows, what it ignores, and that a live event beats a replay.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceStopView } from "@/types/rpc/WorkspaceStopView";

const h = vi.hoisted(() => ({
    live: null as null | ((view: unknown) => void),
    history: vi.fn(async (): Promise<unknown[]> => []),
}));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_name: string, cb: (view: unknown) => void) => {
        h.live = cb;
        return () => {};
    },
}));
vi.mock("@/app/store/mps-history", () => ({ readMuxEventHistory: (...a: unknown[]) => h.history(...(a as [])) }));

import { clockTime, countdown, parseWorkspaceStop, resetWorkspaceStopForTest, startWorkspaceStop, workspaceStop } from "./workspace-stop";

const NOW = Date.parse("2026-11-24T12:00:00Z");
const pending = (inMs: number, over: Partial<WorkspaceStopView> = {}): WorkspaceStopView => ({
    status: "pending",
    stop_at_ms: NOW + inMs,
    reason_id: "timer",
    reason: "nothing has happened for 50 minutes",
    ...over,
});
const settle = () => new Promise((r) => setTimeout(r, 0));

describe("parseWorkspaceStop", () => {
    it("holds a pending stop with its time, its reason id and srv's phrase", () => {
        expect(parseWorkspaceStop(pending(600_000), NOW)).toEqual({ stopAtMs: NOW + 600_000, reasonId: "timer", reason: "nothing has happened for 50 minutes" });
    });

    it("holds nothing for a cleared stop, a missing one, a malformed time, or one long past", () => {
        expect(parseWorkspaceStop({ status: "cleared", stop_at_ms: 0, reason_id: "", reason: "" }, NOW)).toBeNull();
        expect(parseWorkspaceStop(null, NOW)).toBeNull();
        expect(parseWorkspaceStop(undefined, NOW)).toBeNull();
        expect(parseWorkspaceStop(pending(0, { stop_at_ms: Number.NaN }), NOW)).toBeNull();
        expect(parseWorkspaceStop(pending(0, { stop_at_ms: 0 }), NOW)).toBeNull();
        expect(parseWorkspaceStop(pending(-6 * 60_000), NOW)).toBeNull();
        expect(parseWorkspaceStop(pending(-60_000), NOW)).not.toBeNull();
    });
});

describe("the words", () => {
    it("counts down as m:ss and stops at zero", () => {
        expect(countdown(NOW + 600_000, NOW)).toBe("10:00");
        expect(countdown(NOW + 61_000, NOW)).toBe("1:01");
        expect(countdown(NOW + 1, NOW)).toBe("0:01");
        expect(countdown(NOW - 5000, NOW)).toBe("0:00");
    });

    it("reads the stop time as a clock time", () => {
        expect(clockTime(NOW)).toMatch(/\d/);
    });
});

describe("following the event", () => {
    beforeEach(() => {
        resetWorkspaceStopForTest();
        h.live = null;
        h.history.mockReset();
        h.history.mockResolvedValue([]);
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(NOW);
    });
    afterEach(() => vi.useRealTimers());

    it("shows a stop as the event says, and clears it", () => {
        startWorkspaceStop();
        h.live!(pending(600_000));
        expect(workspaceStop()?.reasonId).toBe("timer");
        h.live!({ status: "cleared", stop_at_ms: 0, reason_id: "", reason: "" });
        expect(workspaceStop()).toBeNull();
    });

    it("shows a stop that was held when the window opened", async () => {
        h.history.mockResolvedValue([pending(300_000, { reason_id: "person", reason: "you asked" })]);
        startWorkspaceStop();
        await settle();
        expect(workspaceStop()?.reason).toBe("you asked");
    });

    it("does not let a replay that arrives late undo a live event", async () => {
        let release!: (v: unknown[]) => void;
        h.history.mockReturnValue(new Promise((r) => (release = r)));
        startWorkspaceStop();
        h.live!({ status: "cleared", stop_at_ms: 0, reason_id: "", reason: "" });
        release([pending(300_000)]);
        await settle();
        expect(workspaceStop()).toBeNull();
    });

    it("keeps working when the history cannot be read, and follows once however often it is started", async () => {
        h.history.mockRejectedValue(new Error("no srv"));
        startWorkspaceStop();
        startWorkspaceStop();
        await settle();
        expect(h.history).toHaveBeenCalledTimes(1);
        h.live!(pending(60_000));
        expect(workspaceStop()).not.toBeNull();
    });
});
