// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    views: new Map<string, string>(),
    stack: ["term-1"],
    addWidgetAsPaneTab: vi.fn(async () => {}),
    setActiveBlockInStack: vi.fn(),
    setBlockMeta: vi.fn(async () => {}),
}));

vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: (id: string) => ({ meta: { view: h.views.get(id) } }),
    },
}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: h.setBlockMeta }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForStaticTab: () => ({ getNodeByBlockId: () => ({ id: "node-1", data: { blockStack: h.stack } }) }),
}));
vi.mock("@/layout/lib/layoutStack", () => ({
    addWidgetAsPaneTab: h.addWidgetAsPaneTab,
    effectiveStack: (data: { blockStack: string[] }) => data.blockStack,
    setActiveBlockInStack: h.setActiveBlockInStack,
}));

import { openRemotesInPane, remoteSettingsMenuItems } from "./open-remotes";

describe("openRemotesInPane", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        h.views = new Map([["term-1", "term"]]);
        h.stack = ["term-1"];
    });

    it("adds a Remotes tab to the pane, with the host to expand", async () => {
        await openRemotesInPane("term-1", "db1");
        expect(h.addWidgetAsPaneTab).toHaveBeenCalledWith(expect.anything(), "node-1", {
            meta: { view: "remotes", "remotes:expand": "db1" },
        });
    });

    it("switches to the pane's Remotes tab when it has one", async () => {
        h.views.set("remotes-1", "remotes");
        h.stack = ["term-1", "remotes-1"];
        await openRemotesInPane("term-1", "db1");
        expect(h.addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(h.setBlockMeta).toHaveBeenCalledWith("remotes-1", { "remotes:expand": "db1" });
        expect(h.setActiveBlockInStack).toHaveBeenCalledWith(expect.anything(), "node-1", "remotes-1");
    });
});

describe("remoteSettingsMenuItems", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        h.views = new Map([["term-1", "term"]]);
        h.stack = ["term-1"];
    });

    it("offers nothing for a local pane", () => {
        expect(remoteSettingsMenuItems("term-1", undefined)).toEqual([]);
        expect(remoteSettingsMenuItems("term-1", "")).toEqual([]);
        expect(remoteSettingsMenuItems("term-1", "local")).toEqual([]);
    });

    it("opens Remotes in this pane on the pane's host", async () => {
        const items = remoteSettingsMenuItems("term-1", "db1");
        const item = items.find((i) => i.label === "Remote settings…");
        expect(item).toBeDefined();
        item!.click!();
        await vi.waitFor(() => expect(h.addWidgetAsPaneTab).toHaveBeenCalled());
        expect(h.addWidgetAsPaneTab).toHaveBeenCalledWith(expect.anything(), "node-1", {
            meta: { view: "remotes", "remotes:expand": "db1" },
        });
    });
});
