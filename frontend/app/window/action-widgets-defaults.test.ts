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
    atoms: { fullConfigAtom: (): { widgets: Record<string, unknown> } => ({ widgets: {} }) },
}));
vi.mock("@/store/global", () => store);
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import defaults from "../../../crates/srv/src/config/widgets.json";
import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { getMoreWidgets, getPinnedKeys, openWidget } from "./action-widgets-config";

const wmap = defaults as unknown as Record<string, WidgetConfigType>;
store.atoms.fullConfigAtom = () => ({ widgets: wmap });

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

describe("the singleton flag in the default widget config", () => {
    const flagged = (w: WidgetConfigType) => w.singleton === true;
    const keys = (pred: (w: WidgetConfigType) => boolean) =>
        Object.entries(wmap)
            .filter(([, w]) => pred(w))
            .map(([k]) => k.replace("defwidget@", ""))
            .sort();

    it("marks the management panes", () => {
        expect(keys(flagged)).toEqual(
            ["connectors", "drone", "help", "kanban", "memory", "settings", "swarm", "toolchain", "tower", "vault", "warden"].sort()
        );
    });

    it("leaves the work panes opening a new pane each time", () => {
        for (const key of ["agent", "terminal", "browser", "editor", "files", "sysinfo", "discord", "slack"]) {
            expect(wmap[`defwidget@${key}`]?.singleton, key).toBeUndefined();
        }
    });
});

describe("openWidget", () => {
    it("brings back an open Connectors, Memory or Swarm pane instead of opening another", async () => {
        await openWidget(wmap["defwidget@connectors"]);
        await openWidget(wmap["defwidget@memory"]);
        await openWidget(wmap["defwidget@swarm"]);
        expect(store.openOrFocusPaneByView.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["connectors", "memory", "swarm"]);
        expect(store.createBlock).not.toHaveBeenCalled();
    });

    it("opens a new pane for a singleton on a Shift-click", async () => {
        await openWidget(wmap["defwidget@memory"], { forceNew: true });
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });

    it("opens a new pane for a magnified entry, which asked for a full-size one", async () => {
        await openWidget({ ...wmap["defwidget@memory"], magnified: true });
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });

    it("a user's own singleton: false wins over the view's flag", async () => {
        await openWidget({ ...wmap["defwidget@memory"], singleton: false });
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });

    it("an entry with no flag follows the view it opens (a user's own entry for a moved pane)", async () => {
        const entry = { label: "Mine", blockdef: { meta: { view: "memory" } } } as unknown as WidgetConfigType;
        await openWidget(entry);
        expect(store.openOrFocusPaneByView).toHaveBeenCalledTimes(1);
    });

    it("an installed widget's pane follows the singleton its manifest declares", async () => {
        const pane = (view: string, singleton: boolean) =>
            registerPaneTab({ apiVersion: 1, view, label: view, icon: "puzzle-piece", singleton, create: () => ({}) as never });
        const offA = pane("ext:acme.one/main", true);
        const offB = pane("ext:acme.many/main", false);
        const entry = (view: string) => ({ label: view, blockdef: { meta: { view } } }) as unknown as WidgetConfigType;
        await openWidget(entry("ext:acme.one/main"));
        expect(store.openOrFocusPaneByView).toHaveBeenCalledTimes(1);
        await openWidget(entry("ext:acme.many/main"));
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        offA();
        offB();
    });

    // The Kanban is a pane of its own: its button focuses the one open. An entry that still
    // names the Kanban as a section of Warden or the Swarm (a user's own, from before the moves) does the same.
    it("focuses the open Kanban from its button, and from an entry that names an old section", async () => {
        const entry = (meta: Record<string, unknown>) => ({ label: "Kanban", blockdef: { meta } }) as unknown as WidgetConfigType;
        await openWidget(entry({ view: "kanban" }));
        await openWidget(entry({ view: "swarm", "swarm:section": "kanban" }));
        await openWidget(entry({ view: "warden", "warden:section": "kanban" }));
        expect(store.openOrFocusPaneByView.mock.calls).toEqual([
            ["kanban", { meta: { view: "kanban" } }, undefined],
            ["kanban", { meta: { view: "kanban" } }, undefined],
            ["kanban", { meta: { view: "kanban" } }, undefined],
        ]);
        expect(store.createBlock).not.toHaveBeenCalled();
    });

    it("still opens a new pane for the work panes", async () => {
        await openWidget(wmap["defwidget@terminal"]);
        expect(store.createBlock).toHaveBeenCalledTimes(1);
        await openWidget(wmap["defwidget@agent"]);
        await openWidget(wmap["defwidget@browser"]);
        expect(store.createBlock).toHaveBeenCalledTimes(3);
        expect(store.openOrFocusPaneByView).not.toHaveBeenCalled();
    });
});
