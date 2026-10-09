// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The widget bar a fresh install shows (crates/srv/src/config/widgets.json),
 * and the Armory widget's move to Connectors and Memory (then named Knowledge).
 * SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md §4.3, §4.8.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

const store = vi.hoisted(() => ({
    createBlock: vi.fn(),
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/store/global", () => store);
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import defaults from "../../../crates/srv/src/config/widgets.json";
import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { getMoreWidgets, getPinnedKeys, handleWidgetSelect } from "./action-widgets-config";

const wmap = defaults as unknown as Record<string, WidgetConfigType>;

afterEach(() => {
    store.createBlock.mockClear();
    store.openOrFocusPaneByView.mockClear();
});

describe("the default widget bar", () => {
    it("pins eleven widgets, in the operator's order", () => {
        expect(getPinnedKeys({}, wmap)).toEqual([
            "agent",
            "swarm",
            "memory",
            "files",
            "connectors",
            "terminal",
            "editor",
            "browser",
            "messengers",
            "sysinfo",
            "help",
        ]);
    });

    it("puts the rest in More", () => {
        expect(getMoreWidgets({}, wmap).map((w) => w.key.replace("defwidget@", ""))).toEqual([
            "drone",
            "warden",
            "media",
            "toolchain",
            "tower",
            "settings",
        ]);
    });

    it("gives every widget its own order", () => {
        const orders = Object.values(wmap).map((w) => w["display:order"]);
        expect(new Set(orders).size).toBe(orders.length);
    });

    it("has no Armory widget", () => {
        expect(wmap["defwidget@armory"]).toBeUndefined();
    });
});

describe("a pinned list saved with the Armory", () => {
    it("reads armory as connectors and memory, in its place", () => {
        expect(getPinnedKeys({ "widget:pinned": ["agent", "armory", "sysinfo"] }, wmap)).toEqual([
            "agent",
            "connectors",
            "memory",
            "sysinfo",
        ]);
    });

    it("doesn't list a pane twice when it's already pinned", () => {
        expect(getPinnedKeys({ "widget:pinned": ["knowledge", "armory"] }, wmap)).toEqual(["memory", "connectors"]);
    });
});

describe("handleWidgetSelect", () => {
    it("focuses an open Connectors or Memory pane instead of opening another", async () => {
        await handleWidgetSelect(wmap["defwidget@connectors"]);
        await handleWidgetSelect(wmap["defwidget@memory"]);
        expect(store.openOrFocusPaneByView.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["connectors", "memory"]);
        expect(store.createBlock).not.toHaveBeenCalled();
    });

    it("focuses Memory for a user override that still names the old knowledge view", async () => {
        const unregister = registerPaneTab({ apiVersion: 1, view: "memory", aliases: ["knowledge"], label: "Memory", icon: "brain", create: () => ({}) as never });
        try {
            await handleWidgetSelect({ ...wmap["defwidget@memory"], blockdef: { meta: { view: "knowledge" } } } as WidgetConfigType);
            const [view, blockdef] = store.openOrFocusPaneByView.mock.calls[0] as unknown as [string, BlockDef];
            expect(view).toBe("memory");
            expect(blockdef.meta?.view).toBe("memory");
            expect(store.createBlock).not.toHaveBeenCalled();
        } finally {
            unregister();
        }
    });

    it("still opens a new pane for every other widget", async () => {
        await handleWidgetSelect(wmap["defwidget@terminal"]);
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });
});
