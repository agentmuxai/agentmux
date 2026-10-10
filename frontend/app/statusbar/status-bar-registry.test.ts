// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { registerStatusBarItem, statusBarItems, WIDGET_ORDER_LEFT, WIDGET_ORDER_RIGHT } from "./status-bar-registry";

const item = (id: string, side: "left" | "right", order: number) => ({ id, side, order, render: () => null as never });

describe("the status bar registry", () => {
    it("lays items out by side and order, widget items before the built-ins on the right and after them on the left", () => {
        const off = [
            registerStatusBarItem(item("agentmux:a", "right", 100)),
            registerStatusBarItem(item("widget:x/1", "right", WIDGET_ORDER_RIGHT)),
            registerStatusBarItem(item("agentmux:b", "left", 300)),
            registerStatusBarItem(item("widget:x/2", "left", WIDGET_ORDER_LEFT)),
            registerStatusBarItem(item("agentmux:c", "left", 100)),
        ];
        expect(statusBarItems("right").map((i) => i.id)).toEqual(["widget:x/1", "agentmux:a"]);
        expect(statusBarItems("left").map((i) => i.id)).toEqual(["agentmux:c", "agentmux:b", "widget:x/2"]);
        for (const u of off) u();
        expect(statusBarItems("left")).toEqual([]);
    });

    it("replaces an item registered again, and the old one's removal leaves the new one", () => {
        const first = registerStatusBarItem(item("w", "right", 1));
        const second = registerStatusBarItem(item("w", "right", 2));
        first();
        expect(statusBarItems("right").map((i) => i.order)).toEqual([2]);
        second();
        expect(statusBarItems("right")).toEqual([]);
    });
});
