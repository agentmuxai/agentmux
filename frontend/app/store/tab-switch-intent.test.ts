// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §5.2: setActiveTab publishes
// where it is going before the round trip, and keeps it until the committed
// activeTabId catches up — never just because the RPC returned, since the
// Workspace push can land after the reply.

import { beforeEach, describe, expect, it, vi } from "vitest";

const committed = vi.hoisted(() => ({ set: (() => {}) as (tabId: string) => void }));
const rpc = vi.hoisted(() => ({ calls: [] as { resolve: () => void; reject: (e: Error) => void }[] }));

vi.mock("./window-identity", async () => {
    const { createSignal: signal } = await import("solid-js");
    const [activeTabId, setActiveTabId] = signal("tab-a");
    committed.set = setActiveTabId;
    return {
        workspace: () => ({ oid: "ws-1" }),
        activeTabId,
        windowId: () => "",
        setWindowId: () => {},
        clientId: () => "",
        setClientId: () => {},
        staticTabId: () => "",
        setStaticTabId: () => {},
        client: () => ({}),
        muxWindow: () => ({}),
        tabAtom: () => ({}),
        uiContext: () => ({}),
    };
});
vi.mock("./services", () => ({
    WorkspaceService: {
        SetActiveTab: () => new Promise<void>((resolve, reject) => rpc.calls.push({ resolve, reject })),
        CreateTab: async () => "t",
    },
}));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ keepInactiveTabsLaidOut: () => true }));
vi.mock("./tab-reveal", () => ({
    holdRevealGate: () => {},
    scheduleRevealLift: () => {},
    logUngatedReveal: () => {},
    tabWasShown: () => true,
}));
vi.mock("./focusManager", () => ({ focusManager: { refocusNode: () => {} } }));

import { setActiveTab, switchIntentTabId } from "./tab-actions";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("setActiveTab switch intent", () => {
    beforeEach(async () => {
        rpc.calls.length = 0;
        committed.set("tab-a");
        await settle();
    });

    it("is published before the round trip", () => {
        void setActiveTab("tab-b");
        expect(switchIntentTabId()).toBe("tab-b");
        rpc.calls[0].resolve();
    });

    it("outlives the RPC reply until the committed tab catches up", async () => {
        const done = setActiveTab("tab-b");
        rpc.calls[0].resolve();
        await done;
        expect(switchIntentTabId()).toBe("tab-b");
        committed.set("tab-b");
        expect(switchIntentTabId()).toBeNull();
    });

    it("is dropped when the RPC fails, so the committed tab shows again", async () => {
        const done = setActiveTab("tab-b");
        rpc.calls[0].reject(new Error("rpc down"));
        await expect(done).rejects.toThrow("rpc down");
        expect(switchIntentTabId()).toBeNull();
    });

    it("follows the newest switch; an older one failing doesn't clear it", async () => {
        const first = setActiveTab("tab-b");
        const second = setActiveTab("tab-c");
        expect(switchIntentTabId()).toBe("tab-c");
        rpc.calls[0].reject(new Error("stale"));
        await expect(first).rejects.toThrow("stale");
        expect(switchIntentTabId()).toBe("tab-c");
        rpc.calls[1].resolve();
        await second;
        committed.set("tab-c");
        expect(switchIntentTabId()).toBeNull();
    });

    // ReAgent P1 on #4107: the source tab closed mid-switch and the backend
    // promoted a neighbor instead of the destination.
    it("is dropped when the committed tab moves to a tab outside the switch", async () => {
        const done = setActiveTab("tab-b");
        expect(switchIntentTabId()).toBe("tab-b");
        committed.set("tab-x");
        expect(switchIntentTabId()).toBeNull();
        rpc.calls[0].resolve();
        await done;
    });

    it("survives the committed tab passing through an earlier tab of the same switch", async () => {
        const first = setActiveTab("tab-b");
        const second = setActiveTab("tab-c");
        committed.set("tab-b");
        expect(switchIntentTabId()).toBe("tab-c");
        rpc.calls[0].resolve();
        rpc.calls[1].resolve();
        await Promise.all([first, second]);
        committed.set("tab-c");
        expect(switchIntentTabId()).toBeNull();
    });
});
