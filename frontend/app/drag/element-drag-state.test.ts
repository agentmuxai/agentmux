// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

type MonitorArgs = { onDragStart: () => void; onDrop: () => void };
const monitors: MonitorArgs[] = [];
vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    monitorForElements: (args: MonitorArgs) => {
        monitors.push(args);
        return () => {};
    },
}));

import { elementDragInFlight } from "./element-drag-state";

describe("elementDragInFlight", () => {
    it("is true from drag start until drop, and registers one monitor", () => {
        expect(elementDragInFlight()).toBe(false);
        expect(monitors).toHaveLength(1);

        monitors[0].onDragStart();
        expect(elementDragInFlight()).toBe(true);
        monitors[0].onDrop();
        expect(elementDragInFlight()).toBe(false);

        elementDragInFlight();
        expect(monitors).toHaveLength(1);
    });

    it("clears on a window dragend even when pragmatic's drop never fires", () => {
        monitors[0].onDragStart();
        expect(elementDragInFlight()).toBe(true);
        window.dispatchEvent(new Event("dragend"));
        expect(elementDragInFlight()).toBe(false);
    });
});
