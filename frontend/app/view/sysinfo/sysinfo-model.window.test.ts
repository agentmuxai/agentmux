// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The pane's own window and area: chosen in the Overview, kept in its meta; the window ahead of the global setting. */
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({
    read: vi.fn(async (..._args: unknown[]): Promise<any[]> => []),
    settings: {} as Record<string, unknown>,
}));

vi.mock("@/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ settings: env.settings }) },
    getConnStatusAtom: () => () => ({ status: "connected" }),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { EventReadHistoryCommand: env.read } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { SysinfoViewModel } from "./sysinfo-model";
import { DefaultNumPoints } from "./sysinfo-types";

function model(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => setM({ ...m(), ...patch }));
    const ctx = { blockId: "s1", meta: m, setMeta, isFocused: () => false, visibility: () => "active" };
    return { model: new SysinfoViewModel(ctx as unknown as PaneTabHostContext, "sysinfo"), setMeta };
}

beforeEach(() => {
    env.read.mockClear();
    env.settings = {};
});

afterEach(() => vi.restoreAllMocks());

describe("SysinfoViewModel window", () => {
    it("is the default with nothing chosen, and the global setting when there is one", () => {
        createRoot((dispose) => {
            expect(model().model.numPoints()).toBe(DefaultNumPoints);
            env.settings = { "telemetry:numpoints": 300 };
            expect(model().model.numPoints()).toBe(300);
            dispose();
        });
    });

    it("prefers the window this pane chose over the global setting", () => {
        createRoot((dispose) => {
            env.settings = { "telemetry:numpoints": 300 };
            expect(model({ "graph:numpoints": 60 }).model.numPoints()).toBe(60);
            dispose();
        });
    });

    it("takes a new window at once, saves it as a time, and reads enough history to fill it", async () => {
        await createRoot(async (dispose) => {
            const { model: m, setMeta } = model();
            m.setWindow(900);
            expect(m.numPoints()).toBe(900);
            expect(m.windowSecs()).toBe(900);
            expect(setMeta).toHaveBeenCalledWith({ "graph:windowsecs": 900 });
            await Promise.resolve();
            const last = env.read.mock.calls[env.read.mock.calls.length - 1];
            expect((last[1] as { maxitems: number }).maxitems).toBe(900);
            dispose();
        });
    });

    it("spans the chosen time at any sample interval", () => {
        createRoot((dispose) => {
            env.settings = { "telemetry:interval": 2 };
            const { model: m } = model({ "graph:windowsecs": 300 });
            expect(m.numPoints()).toBe(150);
            expect(m.windowSecs()).toBe(300);
            dispose();
        });
    });

    it("keeps the newest load's history when an older, slower one finishes after it", async () => {
        await createRoot(async (dispose) => {
            const pending: ((items: any[]) => void)[] = [];
            env.read.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
            const { model: m } = model();
            await Promise.resolve();
            m.setWindow(900);
            await Promise.resolve();
            const at = (ts: number) => ({ event: "sysinfo", scopes: ["local"], data: { ts, values: { cpu: 1 } } });
            // The 900-sample load answers first, then the pane's first, shorter one.
            pending[1]([at(1_000), at(2_000), at(3_000)]);
            await new Promise((r) => setTimeout(r, 0));
            pending[0]([at(3_000)]);
            await new Promise((r) => setTimeout(r, 0));
            expect(m.dataAtom().filter((d) => d.blank !== 1)).toHaveLength(3);
            expect(m.loadingAtom()).toBe(false);
            env.read.mockImplementation(async () => []);
            dispose();
        });
    });

    it("keeps a window inside what srv's history holds", () => {
        createRoot((dispose) => {
            const { model: m } = model();
            m.setWindow(5000);
            expect(m.numPoints()).toBe(1024);
            dispose();
        });
    });
});

describe("SysinfoViewModel focus", () => {
    it("shows every area until one is chosen, and reads one saved in the meta", () => {
        createRoot((dispose) => {
            expect(model().model.focus()).toBeNull();
            expect(model({ "sysinfo:focus": "mem" }).model.focus()).toBe("mem");
            expect(model({ "sysinfo:focus": "bogus" }).model.focus()).toBeNull();
            dispose();
        });
    });

    it("takes an area at once, saves it, and is named for it; every area again clears it", () => {
        createRoot((dispose) => {
            const { model: m, setMeta } = model();
            expect(m.viewName()).toBe("Overview");
            m.setFocus("cpu");
            expect(m.focus()).toBe("cpu");
            expect(m.viewName()).toBe("CPU");
            expect(setMeta).toHaveBeenCalledWith({ "sysinfo:focus": "cpu" });
            m.setFocus(null);
            expect(m.viewName()).toBe("Overview");
            expect(setMeta).toHaveBeenLastCalledWith({ "sysinfo:focus": null });
            dispose();
        });
    });

    it("leaves a classic plot type's name alone", () => {
        createRoot((dispose) => {
            expect(model({ "sysinfo:type": "Mem", "sysinfo:focus": "cpu" }).model.viewName()).toBe("Mem");
            dispose();
        });
    });
});
