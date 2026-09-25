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

import { createPaneTabMemory, describePaneTab, prunePaneTabMemory } from "./pane-tab-model";

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

describe("pane tab memory", () => {
    const live = (blockId: string, name: string) =>
        ({ blockId, view: "browser", meta: { view: "browser" }, ordinal: 1, liveViewModel: { viewName: () => name } }) as any;
    const dormant = (blockId: string) =>
        ({ blockId, view: "browser", meta: { view: "browser" }, ordinal: 1, liveViewModel: null }) as any;

    // A re-mounted ViewModel reports the view's generic name ("Browser")
    // until its real title loads. That placeholder must not flash over the
    // remembered page title when the tab becomes active again.
    it("doesn't let a freshly mounted ViewModel's placeholder name replace the remembered one", () => {
        const memory = createPaneTabMemory();
        describePaneTab(live("b1", "A Very Long Page Title"), undefined, memory);
        expect(describePaneTab(dormant("b1"), undefined, memory).label).toBe("A Very Long Page Title");
        // Reactivated: the new ViewModel says "Browser" first...
        expect(describePaneTab(live("b1", "Browser"), undefined, memory).label).toBe("A Very Long Page Title");
        // ...then the real title arrives (and a changed title still updates).
        expect(describePaneTab(live("b1", "Another Page"), undefined, memory).label).toBe("Another Page");
    });

    it("still shows the placeholder when nothing better has been seen yet", () => {
        const memory = createPaneTabMemory();
        expect(describePaneTab(live("b2", "Browser"), undefined, memory).label).toBe("Browser");
    });

    it("keeps a tab's last live name once it goes dormant", () => {
        const memory = createPaneTabMemory();
        describePaneTab(live("b1", "Example Domain"), undefined, memory);
        expect(describePaneTab(dormant("b1"), undefined, memory).label).toBe("Example Domain");
    });

    it("forgets tabs that are no longer in the pane, so it can't grow without bound", () => {
        const memory = createPaneTabMemory();
        describePaneTab(live("b1", "One"), undefined, memory);
        describePaneTab(live("b2", "Two"), undefined, memory);

        prunePaneTabMemory(memory, ["b2"]);

        expect([...memory.names.keys()]).toEqual(["b2"]);
        expect(describePaneTab(dormant("b1"), undefined, memory).label).not.toBe("One");
    });
});
