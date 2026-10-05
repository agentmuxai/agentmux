// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Swarm model's Stats state: loaded with everything else, refreshed when
 *  the panel opens, and kept when a refresh fails. */

import { beforeEach, describe, expect, it, vi } from "vitest";
import * as mos from "@/store/mos";

const rpc = vi.hoisted(() => ({ outcomes: vi.fn() }));

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        {},
        {
            get: (_t, name) => (name === "AmbientOutcomesCommand" ? rpc.outcomes : async () => ({})),
        }
    ),
}));
vi.spyOn(mos, "callBackendService").mockImplementation(async () => []);

import { SwarmViewModel } from "./swarm-model";

beforeEach(() => {
    rpc.outcomes.mockReset();
});

describe("Swarm stats", () => {
    it("loads the counts with everything else, closed", async () => {
        rpc.outcomes.mockResolvedValue({ activity_summary: { accepted: 2 } });
        const vm = new SwarmViewModel("block-1");
        await vm.loadAll();
        expect(vm.ambientOutcomesAtom()).toEqual({ activity_summary: { accepted: 2 } });
        expect(vm.statsOpenAtom()).toBe(false);
        vm.dispose();
    });

    it("opening refreshes the counts; closing doesn't", async () => {
        rpc.outcomes.mockResolvedValue({ activity_summary: { accepted: 2 } });
        const vm = new SwarmViewModel("block-1");
        await vm.loadAll();
        const before = rpc.outcomes.mock.calls.length;
        rpc.outcomes.mockResolvedValue({ activity_summary: { accepted: 5 } });
        vm.toggleStats();
        expect(vm.statsOpenAtom()).toBe(true);
        await vi.waitFor(() => expect(vm.ambientOutcomesAtom()).toEqual({ activity_summary: { accepted: 5 } }));
        vm.toggleStats();
        expect(vm.statsOpenAtom()).toBe(false);
        expect(rpc.outcomes.mock.calls.length).toBe(before + 1);
        vm.dispose();
    });

    it("keeps the last counts when a refresh fails, and stays empty on a server without the command", async () => {
        rpc.outcomes.mockResolvedValue({ next_prompt_suggestion: { accepted: 1 } });
        const vm = new SwarmViewModel("block-1");
        await vm.loadAll();
        rpc.outcomes.mockImplementation(async () => {
            throw new Error("unknown command ambient.outcomes");
        });
        await vm.loadAmbientOutcomes();
        expect(vm.ambientOutcomesAtom()).toEqual({ next_prompt_suggestion: { accepted: 1 } });
        vm.dispose();

        const old = new SwarmViewModel("block-2");
        await old.loadAll();
        expect(old.ambientOutcomesAtom()).toBeNull();
        old.dispose();
    });
});
