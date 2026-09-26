// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Swarm as a native pane tab (Pane Tab contract Phase 2c). */

import { describe, expect, it, vi } from "vitest";

const dispose = vi.fn();
vi.mock("./swarm-model", () => ({
    SwarmViewModel: class {
        constructor(public blockId: string) {}
        dispose = dispose;
    },
}));
vi.mock("./swarm-view", () => ({ SwarmView: () => null }));

import { swarmPaneTab } from "./swarm";

describe("swarmPaneTab", () => {
    it("is native, full-bleed, and takes part in pane zoom", () => {
        expect(swarmPaneTab.view).toBe("swarm");
        expect(swarmPaneTab.create).toBeTypeOf("function");
        expect(swarmPaneTab.capabilities).toEqual({ paneZoom: {}, noPadding: true });
    });

    it("disposes its model — subscriptions and timers — with the instance", () => {
        const inst = swarmPaneTab.create!({ blockId: "w1" } as any);
        expect(dispose).not.toHaveBeenCalled();
        inst.dispose!();
        expect(dispose).toHaveBeenCalledOnce();
    });
});
