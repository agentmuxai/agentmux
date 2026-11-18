// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    widgets: {} as Record<string, unknown>,
    openPane: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/app/store/global", () => ({ atoms: { fullConfigAtom: () => ({ widgets: h.widgets }) } }));
vi.mock("@/app/store/palette-open-pane", () => ({ openPane: h.openPane }));

import { openWidgetCommands, widgetEntries } from "./palette-open-commands";

const open = (id: string) => openWidgetCommands().find((c) => c.id === id)!;

describe("Open Editor, Browser and Drone", () => {
    beforeEach(() => {
        h.widgets = {};
        h.openPane.mockClear();
    });

    it("opens a scratch Editor when widgets.json has no Editor entry", () => {
        void open("open:editor").execute();
        expect(h.openPane).toHaveBeenCalledWith(
            { meta: { view: "editor", "editor:tree_expanded": false, "editor:scratch": true } },
            undefined
        );
    });

    it("opens what the widget bar's own entry opens, so the two make the same pane", () => {
        const blockdef = { meta: { view: "browser", url: "https://example.com" } };
        h.widgets = { "defwidget@browser": { label: "Browser", blockdef } };
        void open("open:browser").execute();
        expect(h.openPane).toHaveBeenCalledWith(blockdef, undefined);
    });

    it("makes a new pane for Shift+Enter", () => {
        void open("open:drone").execute({ forceNew: true });
        expect(h.openPane).toHaveBeenCalledWith({ meta: { view: "drone" } }, true);
    });

    it("is found by the words a person would type", () => {
        expect(open("open:editor").keywords).toContain("new file");
        expect(open("open:browser").label).toBe("Open Browser");
    });
});

describe("one entry for each widget no other entry opens", () => {
    beforeEach(() => {
        h.openPane.mockClear();
        h.widgets = {
            "defwidget@discord": {
                label: "Discord",
                icon: "brands@discord",
                color: "#5865f2",
                description: "Discord, in a pane",
                blockdef: { meta: { view: "browser", url: "https://discord.com/app" } },
            },
            "defwidget@messengers": { label: "Messengers", children: ["discord"] },
            "defwidget@terminal": { label: "Terminal", icon: "terminal", blockdef: { meta: { view: "term" } } },
            "defwidget@browser": { label: "Browser", blockdef: { meta: { view: "browser" } } },
            "mytool": { label: "Build", icon: "hammer", blockdef: { meta: { view: "term", cmd: "npm run build" } } },
            "ext": { label: "Chart", blockdef: { meta: { view: "ext:acme.chart/main" } } },
            // The built-in Agent widget: an empty command is no command, and its view has its own entry.
            "defwidget@agent": {
                label: "Agent",
                blockdef: { meta: { view: "agent", controller: "cmd", cmd: "", "cmd:args": [], "cmd:interactive": true } },
            },
        };
    });

    it("lists a site, a command and a pane of another kind, and nothing the palette already opens (not Agent twice)", () => {
        expect(widgetEntries().map((e) => e.id)).toEqual(["open:widget:discord", "open:widget:mytool", "open:widget:ext"]);
    });

    it("names it Open <label>, with a drawable icon and the widget's colour", () => {
        const discord = widgetEntries()[0];
        expect(discord).toMatchObject({ label: "Open Discord", category: "Open", iconColor: "#5865f2" });
        expect(discord.icon).toBe("up-right-from-square");
        expect(widgetEntries()[1].icon).toBe("hammer");
    });

    it("opens the widget's own blockdef", () => {
        void widgetEntries()[0].execute({ forceNew: true });
        expect(h.openPane).toHaveBeenCalledWith({ meta: { view: "browser", url: "https://discord.com/app" } }, true);
    });

    it("follows the config, so a widget a person adds shows without a reload", () => {
        h.widgets = {};
        expect(widgetEntries()).toEqual([]);
        h.widgets = { late: { label: "Late", blockdef: { meta: { view: "browser", url: "https://example.org" } } } };
        expect(widgetEntries().map((e) => e.label)).toEqual(["Open Late"]);
    });
});
