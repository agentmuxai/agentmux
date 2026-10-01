// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The strip selects a new tab's pill in the frame it first appears — even
// before CreateTab replies — instead of ~150 ms later when the built tab is
// activated. `creatingTabId` is what it shows.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

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

import { cancelTabCreation, createTab, creatingTabId, switchIntentTabId } from "./tab-actions";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("new tab pill selection", () => {
    // createTab imports these lazily; load them up front so two overlapping
    // creations don't race vitest's resolution of the same mocked module.
    beforeAll(async () => {
        await import("@/app/tab/tab-presets");
        await import("@/app/tab/tab-content-settled");
    });

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

    // Codex on #4140: a click on the source tab while the new one builds.
    it("a click in the strip cancels the creation's selection and its activation", async () => {
        createTab();
        state.setTabIds(["tab-a", "tab-new"]);
        expect(creatingTabId()).toBe("tab-new");
        cancelTabCreation();
        expect(creatingTabId()).toBeNull();
        state.create[0].resolve("tab-new");
        await vi.waitFor(() => expect(state.settled).toHaveLength(1));
        expect(creatingTabId()).toBeNull();
        state.settled[0]();
        await settle();
        // Never activated: setActiveTab would have published it as the intent.
        expect(switchIntentTabId()).toBeNull();
    });

    // Codex on #4140: two New Tabs before either pill arrives.
    it("selects the newest of two overlapping creations", async () => {
        createTab();
        createTab();
        state.setTabIds(["tab-a", "tab-1"]);
        expect(creatingTabId()).toBeNull();
        state.setTabIds(["tab-a", "tab-1", "tab-2"]);
        expect(creatingTabId()).toBe("tab-2");
        // Let both creations finish so nothing carries into the next test.
        state.create[0].resolve("tab-1");
        state.create[1].resolve("tab-2");
        await vi.waitFor(() => expect(state.settled.length).toBeGreaterThan(0));
        state.settled.forEach((f) => f());
        await settle();
        state.settled.forEach((f) => f());
        state.setActive("tab-2");
        await settle();
    });

    // ReAgent on #4140: the first pill has arrived, its reply hasn't, when
    // the second New Tab starts.
    it("selects the second pill at once when the first one had already arrived", async () => {
        createTab();
        state.setTabIds(["tab-a", "tab-1"]);
        expect(creatingTabId()).toBe("tab-1");
        createTab();
        state.setTabIds(["tab-a", "tab-1", "tab-2"]);
        expect(creatingTabId()).toBe("tab-2");
        state.create[0].resolve("tab-1");
        state.create[1].resolve("tab-2");
        await vi.waitFor(() => expect(state.settled.length).toBeGreaterThan(0));
        state.settled.forEach((f) => f());
        await settle();
        state.settled.forEach((f) => f());
        state.setActive("tab-2");
        await settle();
    });

    it("a newer New Tab takes over: the older one is left inactive", async () => {
        // setActiveTab publishes its destination as the switch intent at once.
        const intents: (string | null)[] = [];
        createTab();
        state.setTabIds(["tab-a", "tab-1"]);
        state.create[0].resolve("tab-1");
        await vi.waitFor(() => expect(state.settled).toHaveLength(1));
        createTab();
        expect(creatingTabId()).toBeNull();
        state.setTabIds(["tab-a", "tab-1", "tab-2"]);
        expect(creatingTabId()).toBe("tab-2");
        state.settled[0]();
        await settle();
        intents.push(switchIntentTabId());
        state.create[1].resolve("tab-2");
        await vi.waitFor(() => expect(state.settled).toHaveLength(2));
        state.settled[1]();
        await vi.waitFor(() => expect(switchIntentTabId()).toBe("tab-2"));
        expect(intents).toEqual([null]);
    });
});
