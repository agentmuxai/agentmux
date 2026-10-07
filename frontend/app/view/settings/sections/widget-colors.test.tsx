// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Appearance → Widget colors
// (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.5).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
const [settings, setSettings] = createSignal<Record<string, unknown>>({});

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => () => settings()[key],
}));

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { WidgetColorsSettings } from "./widget-colors";

const create = () => ({}) as never;
let unregister: Array<() => void> = [];
beforeEach(() => {
    setConfig.mockReset();
    setSettings({});
    unregister = [
        registerPaneTab({ apiVersion: 1, view: "t-term", aliases: ["t-was"], label: "Terminal", icon: "terminal", defaultHue: 240, create }),
        registerPaneTab({ apiVersion: 1, view: "t-plain", label: "Plain", icon: "square", create }),
        registerPaneTab({ apiVersion: 1, view: "t-legacy", legacyOf: "t-term", label: "Legacy", icon: "square", create }),
    ];
});
afterEach(() => {
    cleanup();
    unregister.forEach((u) => u());
});

const renderSection = () =>
    render(() => <WidgetColorsSettings id="appearance.widget_colors" label="Widget colors" description="d" />);
const row = (label: string) => screen.getByRole("group", { name: `${label} color` }).closest(".setting-row") as HTMLElement;
const swatch = (label: string, name: string) => row(label).querySelector(`[aria-label="${name}"]`) as HTMLElement;
const written = () => setConfig.mock.calls.at(-1)?.[1];

describe("Widget colors", () => {
    it("has a row per registered widget, not for legacy views, and one for a widget registered later", () => {
        renderSection();
        expect(screen.getByRole("group", { name: "Terminal color" })).toBeTruthy();
        expect(screen.getByRole("group", { name: "Plain color" })).toBeTruthy();
        expect(screen.queryByRole("group", { name: "Legacy color" })).toBeNull();
        unregister.push(registerPaneTab({ apiVersion: 1, view: "t-late", label: "Late", icon: "square", create }));
        expect(screen.getByRole("group", { name: "Late color" })).toBeTruthy();
    });

    it("selects the built-in color when unset, None for a widget without one", () => {
        renderSection();
        expect(swatch("Terminal", "Blue").getAttribute("aria-pressed")).toBe("true");
        expect(swatch("Plain", "None").getAttribute("aria-pressed")).toBe("true");
    });

    it("stores a picked color, and None as null", () => {
        renderSection();
        fireEvent.click(swatch("Terminal", "Green"));
        expect(written()).toEqual({ "pane:colors": { "t-term": 120 } });
        fireEvent.click(swatch("Terminal", "None"));
        expect(written()).toEqual({ "pane:colors": { "t-term": null } });
    });

    it("stores nothing when the built-in color is picked back", () => {
        setSettings({ "pane:colors": { "t-term": 120, "t-plain": 30 } });
        renderSection();
        fireEvent.click(swatch("Terminal", "Blue"));
        expect(written()).toEqual({ "pane:colors": { "t-plain": 30 } });
    });

    it("offers Reset only for a widget the user changed, and it removes the key", () => {
        setSettings({ "pane:colors": { "t-plain": 30 } });
        renderSection();
        const reset = (label: string) => screen.getAllByText("Reset").find((b) => row(label).contains(b)) as HTMLElement;
        expect(reset("Terminal").style.visibility).toBe("hidden");
        expect(reset("Plain").style.visibility).toBe("visible");
        fireEvent.click(reset("Plain"));
        expect(written()).toEqual({ "pane:colors": null });
    });

    it("shows a color saved under the widget's former id as set, with Reset", () => {
        setSettings({ "pane:colors": { "t-was": 30 } });
        renderSection();
        expect(swatch("Terminal", "Coral").getAttribute("aria-pressed")).toBe("true");
        const reset = screen.getAllByText("Reset").find((b) => row("Terminal").contains(b)) as HTMLElement;
        expect(reset.style.visibility).toBe("visible");
        fireEvent.click(reset);
        expect(written()).toEqual({ "pane:colors": null });
    });

    it("Reset all deletes the setting, and shows only when something is set", () => {
        renderSection();
        expect(screen.queryByText("Reset all")).toBeNull();
        setSettings({ "pane:colors": { "t-term": 0 } });
        fireEvent.click(screen.getByText("Reset all"));
        expect(written()).toEqual({ "pane:colors": null });
    });

    it("previews the widget's color as a pane-tab pill", () => {
        setSettings({ "pane:colors": { "t-term": 60 } });
        renderSection();
        const chip = row("Terminal").querySelector(".setting-widget-color-preview") as HTMLElement;
        const probe = document.createElement("span");
        probe.style.backgroundColor = paneRoleColor(60, undefined, false, "pill")!;
        expect(chip.style.backgroundColor).toBe(probe.style.backgroundColor);
    });
});
