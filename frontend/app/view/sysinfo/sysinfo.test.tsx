// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo as a native pane tab (Pane Tab contract Phase 2c): it reads and
 * writes its block only through the host context.
 */

import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ settings: {} }) },
    getConnStatusAtom: () => () => ({ status: "connected" }),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { EventReadHistoryCommand: vi.fn(async () => []) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./sysinfo-view", () => ({ SysinfoView: () => null }));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { sysinfoPaneTab } from "./sysinfo";

function fakeCtx(meta: Record<string, unknown>) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        setM({ ...m(), ...patch });
    });
    const ctx: PaneTabHostContext = {
        blockId: "s1",
        meta: m as any,
        setMeta,
        isFocused: () => false,
        visibility: () => "active",
    };
    return { ctx, setMeta };
}

describe("sysinfoPaneTab", () => {
    it("is native, declares the connection capability, and names cpuplot like sysinfo", () => {
        for (const view of ["sysinfo", "cpuplot"] as const) {
            const m = sysinfoPaneTab(view);
            expect(m.view).toBe(view);
            expect(m.create).toBeTypeOf("function");
            expect(m.capabilities?.connection).toBe(true);
            expect(m.label).toBe("Sysinfo");
            expect(m.icon).toBe("chart-line");
        }
    });

    it("titles the pane with its plot type, read from the block's meta, and follows it", () => {
        createRoot((dispose) => {
            const { ctx, setMeta } = fakeCtx({ "sysinfo:type": "Mem" });
            const inst = sysinfoPaneTab("sysinfo").create!(ctx);
            expect(inst.liveTitle!().text).toBe("Mem");
            void setMeta({ "sysinfo:type": "Net" });
            expect(inst.liveTitle!().text).toBe("Net");
            dispose();
        });
    });

    it("defaults to CPU and offers a Plot Type settings menu", () => {
        createRoot((dispose) => {
            const { ctx } = fakeCtx({});
            const inst = sysinfoPaneTab("sysinfo").create!(ctx);
            expect(inst.liveTitle!().text).toBe("CPU");
            expect(inst.settingsMenu!().map((i) => i.label)).toContain("Plot Type");
            dispose();
        });
    });
});
