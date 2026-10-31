// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Pause all agent browsing" is read from srv when the app starts, with or
// without the status bar, and follows srv's event after that.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    list: vi.fn(),
    onEvent: null as null | ((d: { paused: boolean; since_ms: number | null }) => void),
}));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_name: string, fn: typeof h.onEvent) => (h.onEvent = fn) }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { BrowserGrantsListCommand: h.list, BrowserGrantsPauseAllCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { browserPauseAll, startBrowserPauseAll } from "./browser-pause-all";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("browser-pause-all store", () => {
    it("reads a switch a restart left on, tries again when srv isn't answering yet, and follows the event", async () => {
        h.list.mockRejectedValueOnce(new Error("not ready")).mockResolvedValue({ pause_all: { paused: true, since_ms: 5 } });
        vi.spyOn(console, "warn").mockImplementation(() => {});
        startBrowserPauseAll();
        await vi.advanceTimersByTimeAsync(0);
        expect(browserPauseAll().paused).toBe(false);
        await vi.advanceTimersByTimeAsync(3000);
        expect(browserPauseAll()).toEqual({ paused: true, since_ms: 5 });
        h.onEvent!({ paused: false, since_ms: null });
        expect(browserPauseAll().paused).toBe(false);
        // Started again from a status item or Settings, it doesn't read twice.
        startBrowserPauseAll();
        expect(h.list).toHaveBeenCalledTimes(2);
    });
});
