// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    atoms: {
        fullConfigAtom: () => ({
            widgets: {
                "defwidget@slack": { icon: "brands@slack", blockdef: { meta: { view: "browser" } } },
                "defwidget@browser": { icon: "globe", blockdef: { meta: { view: "browser" } } },
                "defwidget@armory": { icon: "vault", label: "Armory", blockdef: { meta: { view: "armory" } } },
                "defwidget@terminal": { icon: "square-terminal", blockdef: { meta: { view: "term" } } },
            },
        }),
    },
}));

import { describePaneTab } from "./pane-tab-model";

const iconFor = (view: string, meta: Record<string, unknown> = {}) =>
    describePaneTab({ blockId: `b-${view}`, view, meta: { view, ...meta }, ordinal: 1, liveViewModel: null }).icon;

describe("pane tab icons", () => {
    it("match the widget-bar icon for the view", () => {
        expect(iconFor("armory")).toEqual({ kind: "fa", name: "vault" });
        expect(iconFor("term")).toEqual({ kind: "fa", name: "square-terminal" });
    });

    it("prefer the view's own widget when several widgets open that view", () => {
        expect(iconFor("browser")).toEqual({ kind: "fa", name: "globe" });
    });

    it("let frame:icon win over the widget icon", () => {
        expect(iconFor("armory", { "frame:icon": "rocket" })).toEqual({ kind: "fa", name: "rocket" });
    });

    it("fall back to the built-in view icon when no widget opens the view", () => {
        expect(iconFor("sysinfo")).toEqual({ kind: "fa", name: "chart-line" });
    });
});

describe("pane tab labels", () => {
    it("fall back to the widget-bar label for the view", () => {
        const tab = describePaneTab({ blockId: "b-a", view: "armory", meta: { view: "armory" }, ordinal: 1, liveViewModel: null });
        expect(tab.label).toBe("Armory");
    });
});
