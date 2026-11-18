// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// #47767: a signal write flushes its dependents at once, and a dependent that throws (a `<Show>` child read after
// its condition went false) makes the write throw. `setActiveTab` wrote three signals before its `try`, so the
// throw skipped the RPC and left the gate held; the switch now goes on, counts the throw and says so in one line.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mockWorkspace = { oid: "ws-1" } as any;

vi.mock("./window-identity", () => ({
    workspace: () => mockWorkspace,
    activeTabId: () => "tab-a",
    windowId: () => "",
    setWindowId: () => {},
    clientId: () => "",
    setClientId: () => {},
    staticTabId: () => "",
    setStaticTabId: () => {},
    client: () => ({}) as any,
    muxWindow: () => ({}) as any,
    tabAtom: () => ({}) as any,
    uiContext: () => ({}) as any,
}));

const setActiveTabRpc = vi.fn(async () => undefined);
vi.mock("./services", () => ({
    WorkspaceService: { SetActiveTab: (...a: unknown[]) => setActiveTabRpc(...(a as [])), CreateTab: async () => "t" },
}));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ keepInactiveTabsLaidOut: () => false }));

const holdRevealGate = vi.fn();
const scheduleRevealLift = vi.fn();
vi.mock("./tab-reveal", () => ({
    holdRevealGate: (id: string) => holdRevealGate(id),
    scheduleRevealLift: () => scheduleRevealLift(),
    logUngatedReveal: () => {},
    tabWasShown: () => false,
}));

import { createMemo, createRoot } from "solid-js";
import { setActiveTab, switchIntentTabId } from "./tab-actions";

describe("setActiveTab when a dependent throws during one of its writes", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 8));
        setActiveTabRpc.mockClear();
        holdRevealGate.mockReset();
        scheduleRevealLift.mockReset();
        vi.spyOn(console, "error").mockImplementation(() => {});
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it("still calls the RPC and lifts the gate, and the one line per switch counts the throw", async () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        holdRevealGate.mockImplementation(() => {
            throw new Error("Stale read from <Show>.");
        });
        await setActiveTab("tab-b");
        expect(setActiveTabRpc).toHaveBeenCalledWith("ws-1", "tab-b");
        expect(scheduleRevealLift).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(50);
        const line = info.mock.calls.map((c) => String(c[0])).find((l) => l.startsWith("[tab-switch] "));
        expect(line).toMatch(/^\[tab-switch\] to=tab-b rpc=\d+ms frames=\d+ms gated=true browser-panes=\d+ agent-panes=\d+ rows=\d+ heap=\S+ window=\S+ threw=1$/);
    });

    it("clears the intent when the RPC fails after the intent's own write threw", async () => {
        let dispose = () => {};
        createRoot((d) => {
            dispose = d;
            createMemo(() => {
                if (switchIntentTabId() === "tab-c") throw new Error("Stale read from <Show>.");
            });
        });
        setActiveTabRpc.mockRejectedValueOnce(new Error("no"));
        await expect(setActiveTab("tab-c")).rejects.toThrow("no");
        expect(switchIntentTabId()).toBeNull();
        dispose();
    });

    it("logs one line for a switch with nothing wrong, and says so when the RPC fails", async () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        await setActiveTab("tab-b");
        vi.advanceTimersByTime(50);
        setActiveTabRpc.mockRejectedValueOnce(new Error("no"));
        await expect(setActiveTab("tab-c")).rejects.toThrow("no");
        vi.advanceTimersByTime(50);
        const lines = info.mock.calls.map((c) => String(c[0])).filter((l) => l.startsWith("[tab-switch] "));
        expect(lines.some((l) => l.includes("to=tab-b") && l.endsWith("threw=0"))).toBe(true);
        expect(lines.some((l) => l.includes("to=tab-c rpc=failed"))).toBe(true);
    });
});
