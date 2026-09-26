// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Drone as a native pane tab (Pane Tab contract Phase 2c). */

import { describe, expect, it, vi } from "vitest";

const dispose = vi.fn();
vi.mock("./drone-model", () => ({
    DroneViewModel: class {
        constructor(public ctx: any) {}
        viewName = () => this.ctx.meta()?.["frame:title"] ?? "Untitled drone";
        dispose = dispose;
    },
}));
vi.mock("./drone-view", () => ({ DroneView: () => null }));

import { dronePaneTab } from "./drone";

describe("dronePaneTab", () => {
    it("is native, full-bleed, and keeps the workflows alias", () => {
        expect(dronePaneTab.view).toBe("drone");
        expect(dronePaneTab.aliases).toEqual(["workflows"]);
        expect(dronePaneTab.create).toBeTypeOf("function");
        expect(dronePaneTab.capabilities).toEqual({ noPadding: true });
    });

    it("titles the pane from its model and disposes the model with the instance", () => {
        const inst = dronePaneTab.create!({ blockId: "d1", meta: () => ({ "frame:title": "Nightly" }) } as any);
        expect(inst.liveTitle!().text).toBe("Nightly");
        inst.dispose!();
        expect(dispose).toHaveBeenCalledOnce();
    });
});
