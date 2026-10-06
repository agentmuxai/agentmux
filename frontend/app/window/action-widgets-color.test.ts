// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.6: a top-bar widget icon is
// tinted with its entry's color, else the color of the view it opens.

import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const [settings, setSettings] = createSignal<Record<string, unknown>>({});
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => () => settings()[key],
}));

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { widgetEntryColor, widgetIconStyle } from "./action-widgets-config";

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

describe("widgetIconStyle", () => {
    it("sets --widget-tint, or nothing so the icon keeps the monochrome token", () => {
        expect(widgetIconStyle(widget({}))).toEqual({ "--widget-tint": paneRoleColor(240, undefined, false, "widgetTint") });
        expect(widgetIconStyle(widget({ blockdef: { meta: { view: "t-none" } } } as Partial<WidgetConfigType>))).toEqual({});
    });
});
