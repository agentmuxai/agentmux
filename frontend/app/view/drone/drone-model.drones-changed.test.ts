// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The drone list follows srv's `drones:changed`, and stops when the pane is disposed. */

import { afterEach, describe, expect, it, vi } from "vitest";

const listDrones = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ListDronesCommand: (...args: unknown[]) => listDrones(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const handlers = new Map<string, () => void>();
const unsubscribed: string[] = [];
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (event: string, handler: () => void) => {
        handlers.set(event, handler);
        return () => {
            unsubscribed.push(event);
            handlers.delete(event);
        };
    },
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { DroneViewModel } from "./drone-model";

const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe("DroneViewModel and drones:changed", () => {
    afterEach(() => {
        listDrones.mockReset();
        handlers.clear();
        unsubscribed.length = 0;
    });

    it("reads the list again when srv says a drone changed, and stops after dispose", async () => {
        listDrones.mockResolvedValue([{ id: "d1", name: "Nightly" }]);
        const model = new DroneViewModel({ blockId: "drone-changed-block", meta: () => ({}) } as unknown as PaneTabHostContext);
        await flush();
        expect(listDrones).toHaveBeenCalledTimes(1);
        expect(model.listAtom()).toEqual([{ id: "d1", name: "Nightly" }]);

        // An agent defines a drone: the open pane lists it without a reopen.
        listDrones.mockResolvedValue([{ id: "d1", name: "Nightly" }, { id: "d2", name: "From an agent" }]);
        handlers.get("drones:changed")!();
        await flush();
        expect(listDrones).toHaveBeenCalledTimes(2);
        expect(model.listAtom().map((d: { id: string }) => d.id)).toEqual(["d1", "d2"]);

        model.dispose();
        expect(unsubscribed).toContain("drones:changed");
        expect(handlers.has("drones:changed")).toBe(false);
    });
});
