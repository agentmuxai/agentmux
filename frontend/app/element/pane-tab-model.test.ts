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

    // A rebuilt browser ViewModel reports stand-in names ("Browser", then the
    // URL hostname) until the page's title arrives, and says so via
    // `viewNameIsPlaceholder`. Stand-ins must not flash over the remembered
    // title when the tab becomes active again.
    const liveP = (blockId: string, name: string, placeholder: boolean) =>
        ({
            blockId,
            view: "browser",
            meta: { view: "browser" },
            ordinal: 1,
            liveViewModel: { viewName: () => name, viewNameIsPlaceholder: () => placeholder },
        }) as any;

    it("doesn't let a rebuilt ViewModel's stand-in names replace the remembered title", () => {
        const memory = createPaneTabMemory();
        describePaneTab(liveP("b1", "A Very Long Page Title", false), undefined, memory);
        expect(describePaneTab(dormant("b1"), undefined, memory).label).toBe("A Very Long Page Title");
        // Reactivated: "Browser", then the hostname, both flagged as stand-ins...
        expect(describePaneTab(liveP("b1", "Browser", true), undefined, memory).label).toBe("A Very Long Page Title");
        expect(describePaneTab(liveP("b1", "agentmux.ai", true), undefined, memory).label).toBe("A Very Long Page Title");
        // ...then the real title arrives (and a real change still updates).
        expect(describePaneTab(liveP("b1", "Another Page", false), undefined, memory).label).toBe("Another Page");
    });

    it("still shows a stand-in when nothing better has been seen yet", () => {
        const memory = createPaneTabMemory();
        expect(describePaneTab(liveP("b2", "agentmux.ai", true), undefined, memory).label).toBe("agentmux.ai");
    });

    it("a real name equal to the view's generic name is NOT treated as a stand-in", () => {
        // e.g. a Swarm tab whose real name is "Swarm": no flag, so it updates.
        const memory = createPaneTabMemory();
        describePaneTab(live("b3", "Something Else"), undefined, memory);
        expect(describePaneTab(live("b3", "Browser"), undefined, memory).label).toBe("Browser");
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
