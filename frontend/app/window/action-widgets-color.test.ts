// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.6: a top-bar widget icon is
// tinted with its entry's color, else the color of the view it opens.

import { createMemo, createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const [settings, setSettings] = createSignal<Record<string, unknown>>({});
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => () => settings()[key],
}));
// A window with no theme of its own shows the default theme setting.
vi.mock("@/app/store/window-appearance", () => ({
    currentWindowTheme: () => (settings()["window:theme"] as string) ?? "default",
}));

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { buildPaneWidgetMenuItems, widgetEntryColor, widgetIconStyle, widgetMenuIconColor } from "./action-widgets-config";

const widget = (overrides: Partial<WidgetConfigType>): WidgetConfigType =>
    ({ "display:order": 0, icon: "circle", label: "W", blockdef: { meta: { view: "t-term" } }, ...overrides }) as WidgetConfigType;

let unregister: () => void;
beforeEach(() => {
    setSettings({});
    unregister = registerPaneTab({ apiVersion: 1, view: "t-term", label: "T", icon: "terminal", defaultHue: 240, create: () => ({}) as never });
});
afterEach(() => unregister());

describe("widgetEntryColor", () => {
    it("is the color of the view the entry opens", () => {
        expect(widgetEntryColor(widget({}), "widgetTint")).toBe(paneRoleColor(240, undefined, false, "widgetTint"));
    });

    it("prefers the entry's own color, as a messenger's brand over its browser view", () => {
        expect(widgetEntryColor(widget({ color: "#5865f2" }), "widgetTint")).toBe(paneRoleColor(undefined, "#5865f2", false, "widgetTint"));
    });

    it("keeps an entry color the palette can't read: as written at full strength, else the view's color", () => {
        for (const color of ["chartreuse", "#f00", "rgb(255, 0, 0)"]) {
            expect(widgetEntryColor(widget({ color }), "identity")).toBe(color);
            expect(widgetEntryColor(widget({ color }), "widgetTint")).toBe(paneRoleColor(240, undefined, false, "widgetTint"));
        }
    });

    it("reads a palette slot's name as that slot", () => {
        expect(widgetEntryColor(widget({ color: "red" }), "widgetTint")).toBe(paneRoleColor(undefined, "red", false, "widgetTint"));
    });

    it("has none for an entry that opens no colored view", () => {
        expect(widgetEntryColor(widget({ blockdef: { meta: { view: "t-none" } } } as Partial<WidgetConfigType>), "widgetTint")).toBeUndefined();
        expect(widgetEntryColor(widget({ blockdef: undefined } as Partial<WidgetConfigType>), "widgetTint")).toBeUndefined();
    });

    it("follows the user's color for the view, and its theme", () => {
        setSettings({ "pane:colors": { "t-term": 120 } });
        expect(widgetEntryColor(widget({}), "widgetTint")).toBe(paneRoleColor(120, undefined, false, "widgetTint"));
        setSettings({ "pane:colors": { "t-term": 120 }, "window:theme": "light" });
        expect(widgetEntryColor(widget({}), "widgetTint")).toBe(paneRoleColor(120, undefined, true, "widgetTint"));
        setSettings({ "pane:colors": { "t-term": null } });
        expect(widgetEntryColor(widget({}), "widgetTint")).toBeUndefined();
    });
});

describe("a widget registered late", () => {
    it("tints its icon once its manifest arrives, without another settings change", () => {
        createRoot((dispose) => {
            const style = createMemo(() => widgetIconStyle(widget({ blockdef: { meta: { view: "t-late" } } } as Partial<WidgetConfigType>)));
            expect(style()).toEqual({});
            const unregisterLate = registerPaneTab({ apiVersion: 1, view: "t-late", label: "L", icon: "square", defaultHue: 120, create: () => ({}) as never });
            expect(style()).toEqual({ "--widget-tint": paneRoleColor(120, undefined, false, "widgetTint") });
            unregisterLate();
            dispose();
        });
    });
});

describe("widgetIconStyle", () => {
    it("sets --widget-tint, or nothing so the icon keeps the monochrome token", () => {
        expect(widgetIconStyle(widget({}))).toEqual({ "--widget-tint": paneRoleColor(240, undefined, false, "widgetTint") });
        expect(widgetIconStyle(widget({ blockdef: { meta: { view: "t-none" } } } as Partial<WidgetConfigType>))).toEqual({});
    });
});

describe("the colour of a widget's icon in the \"+\" menus", () => {
    const entries = {
        "defwidget@term": widget({ label: "Terminal" }),
        "defwidget@plain": widget({ label: "Plain", blockdef: { meta: { view: "t-none" } } } as Partial<WidgetConfigType>),
        "defwidget@chat": widget({ label: "Chat", color: "#5865f2", children: ["kid"] } as Partial<WidgetConfigType>),
        "defwidget@kid": widget({ label: "Kid", color: "#5865f2" }),
    } as Record<string, WidgetConfigType>;
    const build = () => buildPaneWidgetMenuItems(entries, {}, () => {});
    const byLabel = (label: string) => build().find((i) => i.label === label)!;

    it("is the entry's identity colour at full strength, the one its pane header uses", () => {
        expect(widgetMenuIconColor(widget({}))).toBe(paneRoleColor(240, undefined, false, "identity"));
        expect(byLabel("Terminal").iconColor).toBe(paneRoleColor(240, undefined, false, "identity"));
    });

    it("is none for a widget with no colour, so its icon stays plain", () => {
        expect(byLabel("Plain").iconColor).toBeUndefined();
        setSettings({ "pane:colors": { "t-term": null } });
        expect(byLabel("Terminal").iconColor).toBeUndefined();
    });

    it("follows a change in Widget colors the next time the menu is built, with no restart", () => {
        const before = byLabel("Terminal").iconColor;
        setSettings({ "pane:colors": { "t-term": 120 } });
        expect(byLabel("Terminal").iconColor).toBe(paneRoleColor(120, undefined, false, "identity"));
        expect(byLabel("Terminal").iconColor).not.toBe(before);
    });

    it("follows the theme", () => {
        setSettings({ "window:theme": "light" });
        const light = byLabel("Terminal").iconColor;
        setSettings({});
        expect(light).toBeDefined();
        expect(light).not.toBe(byLabel("Terminal").iconColor);
    });

    it("gives a submenu's parent and its entries the same rule, an entry's own colour first", () => {
        const parent = byLabel("Chat");
        expect(parent.type).toBe("submenu");
        expect(parent.iconColor).toBe(paneRoleColor(undefined, "#5865f2", false, "identity"));
        expect(parent.submenu![0].iconColor).toBe(paneRoleColor(undefined, "#5865f2", false, "identity"));
    });
});
