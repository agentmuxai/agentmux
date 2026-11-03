// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The one cross-tab move: a pane's tabs or one pill go to another window tab through a single
 * `RedockFloatingPane` call that names every tab, in order, and the visible one.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    redock: vi.fn(async () => ({ redocked: true })),
    tabs: {} as Record<string, string[]>,
}));

vi.mock("@/store/global", () => ({ atoms: { workspace: () => ({ oid: "ws-1", tabids: Object.keys(hub.tabs) }) } }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    getObjectValue: (oref: string) => ({ blockids: hub.tabs[oref.replace("tab:", "")] }),
}));
vi.mock("@/app/store/services", () => ({ WorkspaceService: { RedockFloatingPane: hub.redock } }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: () => undefined }));
vi.mock("@/layout/lib/layoutNode", () => ({ findNodeByBlockId: () => undefined }));
vi.mock("./layoutModelHooks", () => ({ getLayoutModelForTabById: () => undefined }));
vi.mock("./layoutIntents", () => ({ reconcileWithStoredLayout: () => {} }));

import { stackMovePlan } from "./stackMembers";
import { moveBlocksToTab, tabOfBlock } from "./crossTabDrag";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
    hub.redock.mockClear();
    hub.tabs = { "tab-a": ["a", "b", "c"], "tab-b": ["d"] };
});

describe("moveBlocksToTab", () => {
    it("moves one tab as a pane of its own, with no stack to send", async () => {
        moveBlocksToTab({ blockIds: ["b"], sourceTabId: "tab-a", targetTabId: "tab-b" });
        await settle();
        expect(hub.redock).toHaveBeenCalledWith("b", "tab-a", "ws-1", "tab-b", "ws-1", null, null);
    });

    it("moves a three-tab pane in one call: every tab in order, the visible one named", async () => {
        moveBlocksToTab({ blockIds: ["a", "b", "c"], activeBlockId: "b", sourceTabId: "tab-a", targetTabId: "tab-b" });
        await settle();
        expect(hub.redock).toHaveBeenCalledTimes(1);
        expect(hub.redock).toHaveBeenCalledWith("a", "tab-a", "ws-1", "tab-b", "ws-1", null, null, false, ["a", "b", "c"], "b");
    });

    it("places the first tab at the slot the ghost showed", async () => {
        moveBlocksToTab({ blockIds: ["a", "b"], sourceTabId: "tab-a", targetTabId: "tab-b", targetBlockId: "d", direction: 3 });
        await settle();
        expect(hub.redock).toHaveBeenCalledWith("a", "tab-a", "ws-1", "tab-b", "ws-1", "d", 3, false, ["a", "b"], "a");
    });

    it("joins a pane's tabs when asked to, for one pill or several", async () => {
        moveBlocksToTab({ blockIds: ["b"], sourceTabId: "tab-a", targetTabId: "tab-b", targetBlockId: "d", asTab: true });
        moveBlocksToTab({ blockIds: ["a", "c"], activeBlockId: "c", sourceTabId: "tab-a", targetTabId: "tab-b", targetBlockId: "d", asTab: true });
        await settle();
        expect(hub.redock).toHaveBeenNthCalledWith(1, "b", "tab-a", "ws-1", "tab-b", "ws-1", "d", null, true);
        expect(hub.redock).toHaveBeenNthCalledWith(2, "a", "tab-a", "ws-1", "tab-b", "ws-1", "d", null, true, ["a", "c"], "c");
    });

    it("does nothing for the same tab, no tabs, or a missing tab", async () => {
        moveBlocksToTab({ blockIds: ["a"], sourceTabId: "tab-a", targetTabId: "tab-a" });
        moveBlocksToTab({ blockIds: [], sourceTabId: "tab-a", targetTabId: "tab-b" });
        moveBlocksToTab({ blockIds: ["a"], sourceTabId: "", targetTabId: "tab-b" });
        await settle();
        expect(hub.redock).not.toHaveBeenCalled();
    });
});

describe("tabOfBlock", () => {
    it("finds the window tab that holds a block", () => {
        expect(tabOfBlock("c")).toBe("tab-a");
        expect(tabOfBlock("d")).toBe("tab-b");
        expect(tabOfBlock("nope")).toBeUndefined();
    });
});

describe("stackMovePlan", () => {
    it("a pane without a stack moves as its one block", () => {
        expect(stackMovePlan({ blockId: "x" } as TabLayoutData)).toEqual({ blockIds: ["x"], activeBlockId: "x" });
    });

    it("a stacked pane moves with every tab in order, and the visible one stays visible", () => {
        const data = { blockId: "b", blockStack: ["a", "b", "c"], activeBlockId: "b" } as TabLayoutData;
        expect(stackMovePlan(data)).toEqual({ blockIds: ["a", "b", "c"], activeBlockId: "b" });
    });

    it("falls back to the first tab when the visible one is not a member", () => {
        const data = { blockId: "z", blockStack: ["a", "b"] } as TabLayoutData;
        expect(stackMovePlan(data)).toEqual({ blockIds: ["a", "b"], activeBlockId: "a" });
    });
});
