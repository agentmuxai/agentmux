// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The pane's own window: chosen in the Overview, kept in its meta, ahead of the global setting. */
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

    it("takes a new window at once, saves it, and reads enough history to fill it", async () => {
        await createRoot(async (dispose) => {
            const { model: m, setMeta } = model();
            m.setWindow(900);
            expect(m.numPoints()).toBe(900);
            expect(setMeta).toHaveBeenCalledWith({ "graph:numpoints": 900 });
            await Promise.resolve();
            const last = env.read.mock.calls[env.read.mock.calls.length - 1];
            expect((last[1] as { maxitems: number }).maxitems).toBe(900);
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
