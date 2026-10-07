// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.1–3.2: a pane's own pick,
// then its agent's color, then its widget type's color (Settings, else
// built-in), then none.

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const [settings, setSettings] = createSignal<Record<string, unknown>>({});
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => () => settings()[key],
}));

import { blockRoleColor, isLightThemeActive, resolvePaneIdentity, widgetColorView, widgetHueFor } from "./pane-identity";
import { paneRoleColor } from "./pane-color-scheme";
import { registerPaneTab } from "./pane-tab-registry";

const meta = (m: Record<string, unknown>) => m as Block["meta"];

let unregister: Array<() => void> = [];
beforeEach(() => {
    setSettings({});
    const create = () => ({}) as never;
    unregister = [
        registerPaneTab({ apiVersion: 1, view: "t-term", aliases: ["t-old"], label: "T", icon: "terminal", defaultHue: 240, create }),
        registerPaneTab({ apiVersion: 1, view: "t-plain", label: "P", icon: "square", create }),
        registerPaneTab({ apiVersion: 1, view: "t-legacy", legacyOf: "t-term", label: "L", icon: "square", create }),
    ];
});
afterEach(() => unregister.forEach((u) => u()));

describe("resolvePaneIdentity", () => {
    it("takes the pane's pick over its agent color over its widget color", () => {
        const all = { view: "t-term", "frame:hue": 120, "frame:activebordercolor": "#ff0000" };
        expect(resolvePaneIdentity(meta(all))).toEqual({ hslHue: 120, source: "pick" });
        expect(resolvePaneIdentity(meta({ ...all, "frame:hue": null }))).toEqual({ hex: "#ff0000", source: "agent" });
        expect(resolvePaneIdentity(meta({ view: "t-term" }))).toEqual({ hslHue: 240, source: "widget" });
    });

    it("has no color for a widget type without one, or no view", () => {
        expect(resolvePaneIdentity(meta({ view: "t-plain" }))).toBeUndefined();
        expect(resolvePaneIdentity(meta({}))).toBeUndefined();
        expect(resolvePaneIdentity(undefined)).toBeUndefined();
    });

    it("skips the widget color when asked for identities only", () => {
        expect(resolvePaneIdentity(meta({ view: "t-term" }), { widget: false })).toBeUndefined();
        expect(resolvePaneIdentity(meta({ view: "t-term", "frame:hue": 30 }), { widget: false })?.source).toBe("pick");
    });
});

describe("widgetHueFor", () => {
    it("uses the user's setting over the built-in color, and null as none", () => {
        expect(widgetHueFor("t-term")).toBe(240);
        setSettings({ "pane:colors": { "t-term": 90 } });
        expect(widgetHueFor("t-term")).toBe(90);
        setSettings({ "pane:colors": { "t-term": null } });
        expect(widgetHueFor("t-term")).toBeUndefined();
        setSettings({ "pane:colors": { "t-plain": 300 } });
        expect(widgetHueFor("t-plain")).toBe(300);
    });

    it("resolves an alias to its view, for both the built-in color and the setting", () => {
        expect(widgetHueFor("t-old")).toBe(240);
        setSettings({ "pane:colors": { "t-term": 60 } });
        expect(widgetHueFor("t-old")).toBe(60);
    });

    it("keeps a color the user saved under the view's former id, an alias", () => {
        setSettings({ "pane:colors": { "t-old": 90 } });
        expect(widgetHueFor("t-term")).toBe(90);
        setSettings({ "pane:colors": { "t-old": null } });
        expect(widgetHueFor("t-term")).toBeUndefined();
        // A color saved under the current id wins over the old one.
        setSettings({ "pane:colors": { "t-term": 30, "t-old": 90 } });
        expect(widgetHueFor("t-term")).toBe(30);
    });

    it("gives a legacy view the color of the view it stands in for", () => {
        expect(widgetHueFor("t-legacy")).toBe(240);
        setSettings({ "pane:colors": { "t-term": 60, "t-legacy": 300 } });
        expect(widgetHueFor("t-legacy")).toBe(60);
    });
});

describe("widgetColorView", () => {
    it("is the canonical view, after aliases and legacyOf", () => {
        expect(widgetColorView("t-term")).toBe("t-term");
        expect(widgetColorView("t-old")).toBe("t-term");
        expect(widgetColorView("t-legacy")).toBe("t-term");
        expect(widgetColorView("t-unregistered")).toBe("t-unregistered");
        expect(widgetColorView(undefined)).toBeUndefined();
        expect(widgetColorView("")).toBeUndefined();
    });
});

describe("blockRoleColor", () => {
    it("colors a widget-default pane exactly like a pane with the same pick", () => {
        for (const light of [false, true]) {
            for (const role of ["identity", "border", "pill", "pillActive", "headerTint"] as const) {
                expect(blockRoleColor(meta({ view: "t-term" }), light, role)).toBe(paneRoleColor(240, undefined, light, role));
            }
        }
    });

    it("follows a settings change without being asked again", () => {
        createRoot((dispose) => {
            const color = () => blockRoleColor(meta({ view: "t-term" }), false, "identity");
            expect(color()).toBe(paneRoleColor(240, undefined, false, "identity"));
            setSettings({ "pane:colors": { "t-term": 0 } });
            expect(color()).toBe(paneRoleColor(0, undefined, false, "identity"));
            dispose();
        });
    });
});

describe("isLightThemeActive", () => {
    it("is true only for a light theme id", () => {
        setSettings({ "window:theme": "light" });
        expect(isLightThemeActive()).toBe(true);
        setSettings({ "window:theme": "default" });
        expect(isLightThemeActive()).toBe(false);
        setSettings({});
        expect(isLightThemeActive()).toBe(false);
    });
});
