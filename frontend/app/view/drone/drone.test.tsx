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
import { create } from "./drone-instance";
import { createRoot } from "solid-js";
import { memberMismatch, type PaneTabMember } from "@/app/block/lazy-pane-tab";
import type { PaneTabHostContext, PaneTabInstance, PaneTabManifest } from "@/app/block/pane-tab-registry";

/** The lazy manifest's stand-in has exactly the members the real instance has. */
function membersMatch(manifest: PaneTabManifest, real: PaneTabInstance, ctx: PaneTabHostContext): string[] {
    return createRoot((dispose) => {
        const standIn = manifest.create(ctx);
        const declared = Object.keys(standIn).filter((k) => k !== "component" && k !== "dispose") as PaneTabMember[];
        dispose();
        return memberMismatch(declared, real);
    });
}

describe("dronePaneTab", () => {
    it("is native, full-bleed, and keeps the workflows alias", () => {
        expect(dronePaneTab.view).toBe("drone");
        expect(dronePaneTab.aliases).toEqual(["workflows"]);
        expect(dronePaneTab.create).toBeTypeOf("function");
        expect(dronePaneTab.capabilities).toEqual({ noPadding: true, paneZoom: { baseFontSize: 13 } });
    });

    it("titles the pane from its model and disposes the model with the instance", () => {
        const ctx = { blockId: "d1", meta: () => ({ "frame:title": "Nightly" }) } as any;
        const inst = create(ctx);
        expect(membersMatch(dronePaneTab, inst, ctx)).toEqual([]);
        expect(inst.liveTitle!().text).toBe("Nightly");
        inst.dispose!();
        expect(dispose).toHaveBeenCalledOnce();
    });
});
