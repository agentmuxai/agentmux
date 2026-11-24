// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const settled = vi.hoisted(() => vi.fn());
vi.mock("./settled-pane-rect", () => ({ settledPaneRect: settled }));

import { rectToCreatePageAt } from "./pane-create-rect";

const COLLAPSED = { x: 0, y: 0, width: 0, height: 0 };

describe("rectToCreatePageAt", () => {
    it("makes a pane that can't be seen collapsed, without waiting for it", async () => {
        settled.mockClear();
        expect(await rectToCreatePageAt(() => false, () => ({ x: 1, y: 2, width: 300, height: 200 }))).toEqual(COLLAPSED);
        expect(settled).not.toHaveBeenCalled();
    });

    it("makes a seen pane at the rect its tile settled on", async () => {
        settled.mockImplementation(async (measure: () => unknown) => {
            measure();
            return { x: 0, y: 40, width: 1586, height: 900 };
        });
        expect(await rectToCreatePageAt(() => true, () => ({ x: 0, y: 40, width: 686, height: 900 }))).toEqual({ x: 0, y: 40, width: 1586, height: 900 });
    });

    it("makes it collapsed when the tab was hidden while it waited", async () => {
        let visible = true;
        settled.mockImplementation(async () => {
            visible = false;
            return { x: 0, y: 0, width: 1586, height: 900 };
        });
        expect(await rectToCreatePageAt(() => visible, () => ({ x: 0, y: 0, width: 1586, height: 900 }))).toEqual(COLLAPSED);
    });

    it("makes it collapsed when the pane never got a size, and reads a gone placeholder as empty", async () => {
        settled.mockImplementation(async (measure: () => unknown) => measure());
        expect(await rectToCreatePageAt(() => true, () => null)).toEqual(COLLAPSED);
        expect(await rectToCreatePageAt(() => true, () => ({ x: 5, y: 5, width: 0, height: 0 }))).toEqual(COLLAPSED);
    });
});
