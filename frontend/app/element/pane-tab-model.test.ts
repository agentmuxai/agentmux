// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    atoms: {
        fullConfigAtom: () => ({
            widgets: {
                "defwidget@slack": { icon: "brands@slack", blockdef: { meta: { view: "browser" } } },
                "defwidget@browser": { icon: "globe", blockdef: { meta: { view: "browser" } } },
                "defwidget@memory": { icon: "brain", label: "Memory", blockdef: { meta: { view: "memory" } } },
                "defwidget@terminal": { icon: "square-terminal", blockdef: { meta: { view: "term" } } },
            },
        }),
    },
}));

import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { createPaneTabMemory, describePaneTab, prunePaneTabMemory } from "./pane-tab-model";

// The built-in view icon a tab falls back to comes from the view's manifest.
registerPaneTab(stubPaneTab("sysinfo", { label: "Sysinfo", icon: "chart-line" }));

const iconFor = (view: string, meta: Record<string, unknown> = {}) =>
    describePaneTab({ blockId: `b-${view}`, view, meta: { view, ...meta }, ordinal: 1, liveViewModel: null }).icon;

describe("pane tab icons", () => {
    it("match the widget-bar icon for the view", () => {
        expect(iconFor("memory")).toEqual({ kind: "fa", name: "brain" });
        expect(iconFor("term")).toEqual({ kind: "fa", name: "square-terminal" });
    });

    it("prefer the view's own widget when several widgets open that view", () => {
        expect(iconFor("browser")).toEqual({ kind: "fa", name: "globe" });
    });

    it("let frame:icon win over the widget icon", () => {
        expect(iconFor("memory", { "frame:icon": "rocket" })).toEqual({ kind: "fa", name: "rocket" });
    });

    it("fall back to the built-in view icon when no widget opens the view", () => {
        expect(iconFor("sysinfo")).toEqual({ kind: "fa", name: "chart-line" });
    });
});

describe("pane tab labels", () => {
    it("fall back to the widget-bar label for the view", () => {
        const tab = describePaneTab({ blockId: "b-a", view: "memory", meta: { view: "memory" }, ordinal: 1, liveViewModel: null });
        expect(tab.label).toBe("Memory");
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

describe("a tab's name does not change when it is clicked", () => {
    const tab = (blockId: string, meta: Record<string, unknown>, vm?: Record<string, unknown>) =>
        ({ blockId, view: "browser", meta: { view: "browser", ...meta }, ordinal: 1, liveViewModel: vm ?? null }) as any;
    const liveVm = (name: string, placeholder = false, favicon?: string) => ({
        viewName: () => name,
        viewNameIsPlaceholder: () => placeholder,
        viewFaviconUrl: favicon ? () => favicon : undefined,
    });

    it("a background tab with no memory reads the last real name from its meta, and a click leaves it", () => {
        const memory = createPaneTabMemory(); // a pane that has just mounted
        const meta = { "pane:label": "Release notes", "pane:favicon": "https://example.com/f.ico" };

        const before = describePaneTab(tab("b1", meta), undefined, memory);
        expect(before.label).toBe("Release notes");
        expect(before.icon).toEqual({ kind: "img", src: "https://example.com/f.ico" });

        // Clicked: its ViewModel is built fresh and says "Browser", then the hostname, as stand-ins.
        for (const stand of ["Browser", "example.com"]) {
            const during = describePaneTab(tab("b1", meta, liveVm(stand, true)), undefined, memory);
            expect(during.label).toBe("Release notes");
            expect(during.icon).toEqual(before.icon);
        }
        // The page reports the same title: still the same.
        expect(describePaneTab(tab("b1", meta, liveVm("Release notes")), undefined, memory).label).toBe("Release notes");
    });

    it("clicking through a stack leaves every label as it was", () => {
        const metas: Record<string, Record<string, unknown>> = {
            a: { "pane:label": "Alpha page" },
            b: { "pane:label": "Beta page" },
            c: { "pane:label": "Gamma page" },
        };
        const memory = createPaneTabMemory();
        const labels = (active?: string) =>
            Object.keys(metas).map((id) =>
                describePaneTab(tab(id, metas[id], id === active ? liveVm("Browser", true) : undefined), undefined, memory).label
            );
        const initial = labels();
        expect(initial).toEqual(["Alpha page", "Beta page", "Gamma page"]);
        for (const id of ["b", "c", "a", "c"]) expect(labels(id)).toEqual(initial);
    });

    it("a tab never seen live and with nothing in its meta keeps its fallback until it names itself", () => {
        const memory = createPaneTabMemory();
        const fallback = describePaneTab(tab("b0", {}), undefined, memory).label;
        expect(fallback).not.toBe("example.com");
        // A stand-in is shown while there is nothing better, but it is not something to keep.
        const loading = describePaneTab(tab("b1", {}, liveVm("example.com", true)), undefined, memory);
        expect(loading.label).toBe("example.com");
        expect(loading.persist).toBeUndefined();
    });

    it("asks to keep the real name and favicon it shows, and only when the meta does not hold them", () => {
        const memory = createPaneTabMemory();
        const named = describePaneTab(tab("b1", {}, liveVm("Docs home", false, "https://d.example/f.ico")), undefined, memory);
        expect(named.persist).toEqual({ name: "Docs home", favicon: "https://d.example/f.ico" });

        const kept = describePaneTab(
            tab("b1", { "pane:label": "Docs home", "pane:favicon": "https://d.example/f.ico" }, liveVm("Docs home", false, "https://d.example/f.ico")),
            undefined,
            createPaneTabMemory()
        );
        expect(kept.persist).toBeUndefined();

        const renamed = describePaneTab(tab("b1", { "pane:label": "Docs home" }, liveVm("Docs, v2")), undefined, createPaneTabMemory());
        expect(renamed.label).toBe("Docs, v2");
        expect(renamed.persist).toEqual({ name: "Docs, v2" });
    });

    it("does not keep a name the user or the view's own rule gave: that is stored already", () => {
        const memory = createPaneTabMemory();
        const titled = describePaneTab(tab("b1", { "frame:title": "Mine" }, liveVm("Page")), undefined, memory);
        expect(titled.label).toBe("Mine");
        expect(titled.persist?.name).toBeUndefined();
        const overridden = describePaneTab(tab("b2", {}, liveVm("Page")), "Extra tab", memory);
        expect(overridden.label).toBe("Extra tab");
        expect(overridden.persist?.name).toBeUndefined();
    });

    it("a favicon the tab does not show (its own icon wins) is not kept", () => {
        const withIcon = describePaneTab(tab("b1", { "frame:icon": "rocket" }, liveVm("Page", false, "https://x/f.ico")), undefined, createPaneTabMemory());
        expect(withIcon.persist?.favicon).toBeUndefined();
    });

    it("forgets a closed tab's real-name mark with the rest of its memory", () => {
        const memory = createPaneTabMemory();
        describePaneTab(tab("b1", {}, liveVm("Page")), undefined, memory);
        prunePaneTabMemory(memory, []);
        expect(memory.real.size).toBe(0);
    });
});
