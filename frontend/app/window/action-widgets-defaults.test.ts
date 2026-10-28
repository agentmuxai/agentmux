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
    // SPEC_WORKSPACES_2026_10_23.md §3.1.
    it("pins eight widgets, in the operator's order", () => {
        expect(getPinnedKeys({}, wmap)).toEqual([
            "agent",
            "swarm",
            "memory",
            "connectors",
            "files",
            "browser",
            "settings",
            "help",
        ]);
    });

    it("puts the rest in More", () => {
        expect(getMoreWidgets({}, wmap).map((w) => w.key.replace("defwidget@", ""))).toEqual([
            "kanban",
            "terminal",
            "editor",
            "messengers",
            "sysinfo",
            "drone",
            "warden",
            "vault",
            "toolchain",
            "tower",
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

describe("handleWidgetSelect", () => {
    it("focuses an open Connectors or Memory pane instead of opening another", async () => {
        await handleWidgetSelect(wmap["defwidget@connectors"]);
        await handleWidgetSelect(wmap["defwidget@memory"]);
        expect(store.openOrFocusPaneByView.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["connectors", "memory"]);
        expect(store.createBlock).not.toHaveBeenCalled();
    });

    // The Kanban is a section of the Swarm: its button switches the Swarm in this tab to it. An entry
    // that still names Warden's Kanban (a user's own, from before the move) does the same.
    it("opens the Kanban's button on the Swarm's Kanban section, in the Swarm already open", async () => {
        const entry = (meta: Record<string, unknown>) => ({ label: "Kanban", blockdef: { meta } }) as unknown as WidgetConfigType;
        await handleWidgetSelect(entry({ view: "swarm", "swarm:section": "kanban" }));
        await handleWidgetSelect(entry({ view: "warden", "warden:section": "kanban" }));
        expect(store.openOrFocusPaneByView.mock.calls).toEqual([
            ["swarm", { meta: { view: "swarm", "swarm:section": "kanban" } }, { "swarm:section": "kanban" }],
            ["swarm", { meta: { view: "swarm", "swarm:section": "kanban" } }, { "swarm:section": "kanban" }],
        ]);
        expect(store.createBlock).not.toHaveBeenCalled();
    });

    it("still opens a new pane for every other widget", async () => {
        // The Swarm's own button too: each opens another Swarm.
        await handleWidgetSelect(wmap["defwidget@swarm"]);
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        store.createBlock.mockClear();
        await handleWidgetSelect(wmap["defwidget@terminal"]);
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });
});
