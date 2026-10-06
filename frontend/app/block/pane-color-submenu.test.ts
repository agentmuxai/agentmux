// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Pane Color menu's "Use for all <Label> panes" entry
// (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.7).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const applyHueToAllPanes = vi.fn();
vi.mock("./pane-color-menu", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./pane-color-menu")>()),
    applyHueToAllPanes: (...args: unknown[]) => applyHueToAllPanes(...args),
}));

import { buildPaneColorSubmenu } from "./blockframe";
import { registerPaneTab } from "./pane-tab-registry";

const block = (meta: Record<string, unknown>) => ({ oid: "block-1", meta }) as unknown as Block;
const entries = (meta: Record<string, unknown>) =>
    (buildPaneColorSubmenu(block(meta)).find((i) => i.label === "Pane Color")?.submenu ?? []) as ContextMenuItem[];
const useForAll = (meta: Record<string, unknown>) => entries(meta).find((i) => i.label?.startsWith("Use for all"));

let unregister: Array<() => void> = [];
beforeEach(() => {
    applyHueToAllPanes.mockClear();
    const create = () => ({}) as never;
    unregister = [
        registerPaneTab({ apiVersion: 1, view: "t-term", aliases: ["t-old"], label: "Terminal", icon: "terminal", create }),
    ];
});
afterEach(() => unregister.forEach((u) => u()));

describe("Use for all <Label> panes", () => {
    it("names the widget and makes this pane's pick its color", () => {
        const item = useForAll({ view: "t-old", "frame:hue": 270 });
        expect(item?.label).toBe("Use for all Terminal panes");
        item!.click!();
        expect(applyHueToAllPanes).toHaveBeenCalledWith("block-1", "t-term", 270);
    });

    it("is not offered when the pane has no pick of its own", () => {
        expect(useForAll({ view: "t-term" })).toBeUndefined();
        expect(useForAll({ view: "t-term", "frame:hue": null })).toBeUndefined();
    });

    it("is not offered on an agent pane", () => {
        expect(useForAll({ view: "t-term", "frame:hue": 270, agentId: "agent-1" })).toBeUndefined();
    });
});
