// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Pane Color menu's "Use for all <Label> panes" entry
// (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.7).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const applyColorToAllPanes = vi.fn();
vi.mock("./pane-color-menu", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./pane-color-menu")>()),
    applyColorToAllPanes: (...args: unknown[]) => applyColorToAllPanes(...args),
}));

import { buildPaneColorSubmenu } from "./blockframe";
import { registerPaneTab } from "@/app/store/pane-tab-registry";

const block = (meta: Record<string, unknown>) => ({ oid: "block-1", meta }) as unknown as Block;
const entries = (meta: Record<string, unknown>) =>
    (buildPaneColorSubmenu(block(meta)).find((i) => i.label === "Pane Color")?.submenu ?? []) as ContextMenuItem[];
const useForAll = (meta: Record<string, unknown>) => entries(meta).find((i) => i.label?.startsWith("Use for all"));

let unregister: Array<() => void> = [];
beforeEach(() => {
    applyColorToAllPanes.mockClear();
    const create = () => ({}) as never;
    unregister = [
        registerPaneTab({ apiVersion: 1, view: "t-term", aliases: ["t-old"], label: "Terminal", icon: "terminal", create }),
    ];
});
afterEach(() => unregister.forEach((u) => u()));

describe("the Pane Color menu", () => {
    const shape = (meta: Record<string, unknown>) => entries(meta).map((i) => (i.type === "separator" ? "-" : i.label));
    const HUES = ["Rose", "Red", "Orange", "Amber", "Yellow", "Green", "Jade", "Teal", "Sky", "Blue", "Violet", "Magenta"];

    it("is Default, the 12 hues, then Black and White in a group of their own", () => {
        expect(shape({ view: "t-term" })).toEqual(["Default", "-", ...HUES, "-", "Black", "White"]);
    });

    it("gives every colour a swatch, Black and White as themselves", () => {
        const items = entries({ view: "t-term" }).filter((i) => i.type !== "separator" && i.label !== "Default");
        expect(items).toHaveLength(14);
        for (const item of items) expect(item.swatchColor, item.label).toMatch(/^#[0-9a-f]{6}$/);
        expect(items.slice(12).map((i) => i.swatchColor)).toEqual(["#000000", "#ffffff"]);
    });

    it("checks Default with no pick, the picked slot otherwise, and the slot a retired id became", () => {
        const checked = (meta: Record<string, unknown>) => entries(meta).filter((i) => i.checked).map((i) => i.label);
        expect(checked({ view: "t-term" })).toEqual(["Default"]);
        expect(checked({ view: "t-term", "frame:color": "black" })).toEqual(["Black"]);
        expect(checked({ view: "t-term", "frame:color": "azure" })).toEqual(["Blue"]);
        expect(checked({ view: "t-term", "frame:color": "indigo" })).toEqual(["Violet"]);
    });
});

describe("Use for all <Label> panes", () => {
    it("names the widget and makes this pane's pick its color", () => {
        const item = useForAll({ view: "t-old", "frame:color": "violet" });
        expect(item?.label).toBe("Use for all Terminal panes");
        item!.click!();
        expect(applyColorToAllPanes).toHaveBeenCalledWith("block-1", "t-term", "violet");
    });

    it("reads an older HSL pick as the slot nearest it", () => {
        const item = useForAll({ view: "t-old", "frame:hue": 270 });
        item!.click!();
        expect(applyColorToAllPanes).toHaveBeenCalledWith("block-1", "t-term", "violet");
    });

    it("is not offered when the pane has no pick of its own", () => {
        expect(useForAll({ view: "t-term" })).toBeUndefined();
        expect(useForAll({ view: "t-term", "frame:color": null, "frame:hue": null })).toBeUndefined();
    });

    it("is not offered on an agent pane", () => {
        expect(useForAll({ view: "t-term", "frame:color": "violet", agentId: "agent-1" })).toBeUndefined();
    });
});
