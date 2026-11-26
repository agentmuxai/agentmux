// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md §6.4: with
// `window:keepinactivetabslaidout`, switching to a tab that has already been
// shown skips the reveal gate (nothing is left to hide: measured on #3686);
// a tab's first reveal, and every switch with the setting off, stay gated.

import { beforeEach, describe, expect, it, vi } from "vitest";

let mockActiveTabId = "tab-a";
const mockWorkspace = { oid: "ws-1" } as any;

// Same exact-shape mock as tab-actions.test.ts: global.ts destructures all of
// these off window-identity at import time.
vi.mock("./window-identity", () => ({
    workspace: () => mockWorkspace,
    activeTabId: () => mockActiveTabId,
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

vi.mock("./services", () => ({
    WorkspaceService: { SetActiveTab: async () => undefined, CreateTab: async () => "t" },
}));

let keepLaidOut = false;
vi.mock("@/app/workspace/window-tab-visibility", () => ({
    keepInactiveTabsLaidOut: () => keepLaidOut,
}));

const holdRevealGate = vi.fn();
const scheduleRevealLift = vi.fn();
const logUngatedReveal = vi.fn();
const shown = new Set<string>();
vi.mock("./tab-reveal", () => ({
    holdRevealGate: (id: string) => holdRevealGate(id),
    scheduleRevealLift: () => scheduleRevealLift(),
    logUngatedReveal: (id: string) => logUngatedReveal(id),
    tabWasShown: (id: string) => shown.has(id),
    whenGateLifted: (fn: () => void) => fn(),
}));

import { setActiveTab } from "./tab-actions";

describe("setActiveTab reveal gate", () => {
    beforeEach(() => {
        mockActiveTabId = "tab-a";
        keepLaidOut = false;
        shown.clear();
        holdRevealGate.mockClear();
        scheduleRevealLift.mockClear();
        logUngatedReveal.mockClear();
    });

    it("gates every switch with the setting off, even to a tab already shown", async () => {
        shown.add("tab-b");
        await setActiveTab("tab-b");
        expect(holdRevealGate).toHaveBeenCalledWith("tab-b");
        expect(scheduleRevealLift).toHaveBeenCalled();
        expect(logUngatedReveal).not.toHaveBeenCalled();
    });

    it("gates a tab's first reveal with the setting on", async () => {
        keepLaidOut = true;
        await setActiveTab("tab-b");
        expect(holdRevealGate).toHaveBeenCalledWith("tab-b");
        expect(scheduleRevealLift).toHaveBeenCalled();
    });

    it("skips the gate for a tab already shown and kept laid out, and logs the reveal", async () => {
        keepLaidOut = true;
        shown.add("tab-b");
        await setActiveTab("tab-b");
        expect(holdRevealGate).not.toHaveBeenCalled();
        expect(scheduleRevealLift).not.toHaveBeenCalled();
        expect(logUngatedReveal).toHaveBeenCalledWith("tab-b");
    });

    it("logs the swap line for a tab already shown, once the swap is painted", async () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        keepLaidOut = true;
        shown.add("tab-b");
        await setActiveTab("tab-b");
        const swapLines = () => info.mock.calls.map((c) => String(c[0])).filter((l) => l.startsWith("[perf] tab-switch-swap"));
        expect(swapLines()).toEqual([]);
        for (let i = 0; i < 6 && frames.length > 0; i++) frames.splice(0).forEach((cb) => cb(0));
        expect(swapLines()).toHaveLength(1);
        expect(swapLines()[0]).toMatch(/^\[perf\] tab-switch-swap to=tab-b kind=warm swap=\d+ms longest-task=(n\/a|\d+ms)$/);
        info.mockRestore();
        vi.unstubAllGlobals();
    });

    it("logs the swap line as cold for a tab the reveal gates", async () => {
        const frames: FrameRequestCallback[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        keepLaidOut = true;
        await setActiveTab("tab-b");
        for (let i = 0; i < 6 && frames.length > 0; i++) frames.splice(0).forEach((cb) => cb(0));
        const lines = info.mock.calls.map((c) => String(c[0])).filter((l) => l.startsWith("[perf] tab-switch-swap"));
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("kind=cold");
        info.mockRestore();
        vi.unstubAllGlobals();
    });
});
