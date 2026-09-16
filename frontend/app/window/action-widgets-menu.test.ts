// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { buildWidgetOpenActions } from "./action-widgets-menu";

const openNewWindowWithView = vi.fn();
const rpcCall = vi.fn();
const openViewInNewTab = vi.fn();

vi.mock("@/store/global", () => ({
    getApi: () => ({ openNewWindowWithView }),
}));
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: (...args: unknown[]) => rpcCall(...args) },
}));
vi.mock("@/app/tab/tab-presets", () => ({
    openViewInNewTab: (...args: unknown[]) => openViewInNewTab(...args),
}));

function widget(overrides: Partial<WidgetConfigType> = {}): WidgetConfigType {
    return {
        "display:order": 0,
        "display:pinned": false,
        icon: "circle",
        label: "Widget",
        blockdef: { meta: { view: "browser" } },
        ...overrides,
    } as WidgetConfigType;
}

describe("buildWidgetOpenActions", () => {
    it("returns an empty array when the widget doesn't resolve", () => {
        expect(buildWidgetOpenActions("nope", {})).toEqual([]);
    });

    it("returns an empty array when the widget has no resolvable view", () => {
        const wmap = { "defwidget@browser": widget({ blockdef: {} }) };
        expect(buildWidgetOpenActions("browser", wmap)).toEqual([]);
    });

    it("returns exactly the three open actions, in order, for a leaf widget", () => {
        const wmap = { "defwidget@browser": widget() };
        const actions = buildWidgetOpenActions("browser", wmap);
        expect(actions.map((a) => a.label)).toEqual([
            "Open in New Window",
            "Open in Floating Pane",
            "Open in New Tab",
        ]);
    });

    it("'Open in New Window' calls getApi().openNewWindowWithView with the resolved view + meta", () => {
        const wmap = { "defwidget@terminal": widget({ blockdef: { meta: { view: "term" } } }) };
        buildWidgetOpenActions("terminal", wmap)[0].run();
        expect(openNewWindowWithView).toHaveBeenCalledWith("term", { view: "term" });
    });

    it("'Open in Floating Pane' calls pane.open with floating: true", () => {
        const wmap = { "defwidget@editor": widget({ blockdef: { meta: { view: "editor" } } }) };
        buildWidgetOpenActions("editor", wmap)[1].run();
        expect(rpcCall).toHaveBeenCalledWith(
            "pane.open",
            { view: "editor", meta: { view: "editor" }, floating: true },
            {}
        );
    });

    it("'Open in New Tab' calls openViewInNewTab with the resolved view + meta", () => {
        const wmap = { "defwidget@sysinfo": widget({ blockdef: { meta: { view: "sysinfo" } } }) };
        buildWidgetOpenActions("sysinfo", wmap)[2].run();
        expect(openViewInNewTab).toHaveBeenCalledWith("sysinfo", { view: "sysinfo" });
    });
});
