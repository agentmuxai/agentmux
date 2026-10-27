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
vi.mock("./swarm-root", () => ({ SwarmView: () => null }));

import { swarmPaneTab } from "./swarm";
import { create } from "./swarm-instance";
import { createRoot, createSignal } from "solid-js";
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

describe("swarmPaneTab", () => {
    it("is native, full-bleed, and takes part in pane zoom", () => {
        expect(swarmPaneTab.view).toBe("swarm");
        expect(swarmPaneTab.create).toBeTypeOf("function");
        expect(swarmPaneTab.capabilities).toEqual({ paneZoom: {}, noPadding: true });
    });

    it("declares the Supervisor and Broadcast sections, opening on the Supervisor", () => {
        expect(swarmPaneTab.sections?.default).toBe("supervisor");
        expect(swarmPaneTab.sections?.items.map((s) => s.id)).toEqual(["supervisor", "broadcast"]);
    });

    it("is named by the selected section", () => {
        const [meta, setMeta] = createSignal<Record<string, unknown>>({});
        createRoot((dispose) => {
            const inst = create({ blockId: "w1", meta } as any);
            expect(inst.liveTitle!().text).toBe("Swarm · Supervisor");
            setMeta({ "swarm:section": "broadcast" });
            expect(inst.liveTitle!().text).toBe("Swarm · Broadcast");
            setMeta({ "swarm:section": "nope" });
            expect(inst.liveTitle!().text).toBe("Swarm · Supervisor");
            dispose();
        });
    });

    it("disposes its model — subscriptions and timers — with the instance", () => {
        const ctx = { blockId: "w1", meta: () => ({}) } as any;
        const inst = create(ctx);
        expect(membersMatch(swarmPaneTab, inst, ctx)).toEqual([]);
        expect(dispose).not.toHaveBeenCalled();
        inst.dispose!();
        expect(dispose).toHaveBeenCalledOnce();
    });
});
