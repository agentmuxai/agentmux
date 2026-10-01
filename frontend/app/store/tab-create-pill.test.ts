// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The strip selects a new tab's pill in the frame it first appears — even
// before CreateTab replies — instead of ~150 ms later when the built tab is
// activated. `creatingTabId` is what it shows.

import { beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({
    setActive: (() => {}) as (tabId: string) => void,
    setTabIds: (() => {}) as (tabIds: string[]) => void,
    create: [] as { resolve: (id: string) => void; reject: (e: Error) => void }[],
    settled: [] as (() => void)[],
}));

vi.mock("./window-identity", async () => {
    const { createSignal } = await import("solid-js");
    const [activeTabId, setActive] = createSignal("tab-a");
    const [tabIds, setTabIds] = createSignal<string[]>(["tab-a"]);
    state.setActive = setActive;
    state.setTabIds = setTabIds;
    return {
        workspace: () => ({ oid: "ws-1", tabids: tabIds(), pinnedtabids: [] }),
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
        CreateTab: () => new Promise<string>((resolve, reject) => state.create.push({ resolve, reject })),
        SetActiveTab: async () => undefined,
    },
}));
vi.mock("@/app/tab/tab-presets", () => ({ applyTabPreset: async () => {}, DEFAULT_TAB_PRESET: {} }));
vi.mock("@/app/tab/tab-content-settled", () => ({
    whenTabContentSettled: () => new Promise<boolean>((resolve) => state.settled.push(() => resolve(true))),
}));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ keepInactiveTabsLaidOut: () => true }));
vi.mock("./focusManager", () => ({ focusManager: { refocusNode: () => {} } }));

import { createTab, creatingTabId } from "./tab-actions";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("new tab pill selection", () => {
    beforeEach(async () => {
        state.create.length = 0;
        state.settled.length = 0;
        state.setActive("tab-a");
        state.setTabIds(["tab-a"]);
        await settle();
    });

    it("selects the new tab as soon as the workspace shows it, before CreateTab replies", () => {
        createTab();
        expect(creatingTabId()).toBeNull();
        state.setTabIds(["tab-a", "tab-new"]);
        expect(creatingTabId()).toBe("tab-new");
        state.create[0].resolve("tab-new");
    });

    it("keeps it selected while the tab builds, until the committed tab reaches it", async () => {
        createTab();
        state.setTabIds(["tab-a", "tab-new"]);
        state.create[0].resolve("tab-new");
        await vi.waitFor(() => expect(state.settled).toHaveLength(1));
        expect(creatingTabId()).toBe("tab-new");
        state.settled[0]();
        await settle();
        expect(creatingTabId()).toBe("tab-new");
        state.setActive("tab-new");
        expect(creatingTabId()).toBeNull();
    });

    it("lets go as soon as the user goes to another tab meanwhile", async () => {
        createTab();
        state.setTabIds(["tab-a", "tab-b", "tab-new"]);
        state.create[0].resolve("tab-new");
        await vi.waitFor(() => expect(state.settled).toHaveLength(1));
        state.setActive("tab-b");
        expect(creatingTabId()).toBeNull();
        state.settled[0]();
        await settle();
        expect(creatingTabId()).toBeNull();
    });

    it("lets go when the creation fails", async () => {
        createTab();
        state.create[0].reject(new Error("rpc down"));
        await settle();
        expect(creatingTabId()).toBeNull();
    });
});
