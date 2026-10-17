// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Fleet actions on a selection that includes agents of other instances: an
 * action sends only what it can reach, reports every other target with its
 * reason, and a remote agent that goes away leaves the selection.
 * docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §4, §5.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as mos from "@/store/mos";

const rpc = vi.hoisted(() => ({
    broadcast: vi.fn(),
    stop: vi.fn(),
    other: vi.fn(),
}));

vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: vi.fn((_sub: unknown) => () => {}) };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        mock.muxEventSubscribe({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        {},
        {
            get: (_t, name) => {
                if (name === "FleetBroadcastCommand") return rpc.broadcast;
                if (name === "FleetBulkStopCommand") return rpc.stop;
                if (name === "SwarmOtherInstancesCommand") return rpc.other;
                return async () => ({});
            },
        }
    ),
}));
vi.spyOn(mos, "callBackendService").mockImplementation(async () => []);

import { SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";

const LOAP = "blk-loap"; // another channel on this machine: has a block id
const MANOZ = "remote:lan:area54#1/stable/Manoz";
const OPAZ = "remote:lan:area54#1/stable/Opaz";
const ROOK = "remote:cloud:home#3/stable/Rook"; // a cloud install: nothing reaches it yet

const both: SwarmOtherInstances = {
    hostname: "narko",
    channel: "stable",
    hosts: [
        {
            host_id: "host:narko",
            display_name: "narko",
            tier: "host",
            channels: [{ channel: "dev", seen_at_ms: 1, stale: false, os: "windows", agents: [{ name: "Loap", block_id: LOAP }] }],
        },
        {
            host_id: "lan:area54#1",
            display_name: "Area54",
            tier: "lan",
            channels: [
                { channel: "stable", seen_at_ms: 1, stale: false, os: "macos", agents: [{ name: "Manoz", block_id: "" }, { name: "Opaz", block_id: "" }] },
            ],
        },
        {
            host_id: "cloud:home#3",
            display_name: "home",
            tier: "cloud",
            channels: [{ channel: "stable", seen_at_ms: 1, stale: false, os: "linux", agents: [{ name: "Rook", block_id: "" }] }],
        },
    ],
};

/** This instance's own agents, as the tree lists them. */
const LOCAL = ["blk-a", "blk-b"];

async function modelWith(data: SwarmOtherInstances) {
    rpc.other.mockResolvedValue(data);
    const vm = new SwarmViewModel("block-1");
    vi.spyOn(vm, "buildTree").mockReturnValue(LOCAL.map((blockId) => ({ blockId, agentName: blockId })) as ReturnType<SwarmViewModel["buildTree"]>);
    await vm.loadOtherInstances();
    return vm;
}

beforeEach(() => {
    rpc.broadcast.mockReset().mockImplementation(async (_client: unknown, { targets }: { targets: string[] }) => ({
        succeeded: targets,
        failed: [],
        aborted_early: false,
    }));
    rpc.stop.mockReset().mockImplementation(async (_client: unknown, { targets }: { targets: string[] }) => ({
        succeeded: targets,
        failed: [],
        aborted_early: false,
    }));
    rpc.other.mockReset();
});

afterEach(() => vi.useRealTimers());

describe("broadcast to a mixed selection", () => {
    // A LAN peer's agents get the broadcast as a message signed by this
    // AgentMux (docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §4.2).
    it("sends the agents it can reach, LAN peers' included, and reports the others with the reason", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([LOAP, MANOZ, OPAZ, ROOK], true);

        const result = await vm.broadcastToSelection("merge on approval");

        expect(rpc.broadcast).toHaveBeenCalledTimes(1);
        expect(rpc.broadcast).toHaveBeenCalledWith({}, { targets: [LOAP, MANOZ, OPAZ], message: "merge on approval" });
        expect(result.succeeded).toEqual([LOAP, MANOZ, OPAZ]);
        expect(result.failed).toEqual([{ id: ROOK, error: "can't message: no verified link to that machine yet" }]);
    });

    it("names every target in the result, so the panel never prints a block id or a key", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([LOAP, MANOZ], true);
        await vm.broadcastToSelection("hi");
        expect(vm.lastFleetResultAtom()!.labels).toEqual({
            [LOAP]: "Loap · narko · dev",
            [MANOZ]: "Manoz · Area54",
        });
    });

    it("sends nothing at all when no selected agent can be messaged", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([ROOK], true);
        const result = await vm.broadcastToSelection("hi");
        expect(rpc.broadcast).not.toHaveBeenCalled();
        expect(result.succeeded).toEqual([]);
        expect(result.failed).toHaveLength(1);
    });

    it("sends an all-local selection exactly as before", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", "blk-b"], true);
        const result = await vm.broadcastToSelection("hi");
        expect(rpc.broadcast).toHaveBeenCalledWith({}, { targets: ["blk-a", "blk-b"], message: "hi" });
        expect(result.failed).toEqual([]);
    });

    it("doesn't send to a key the Swarm no longer lists, and says so", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", "blk-gone"], true);
        const result = await vm.broadcastToSelection("hi");
        expect(rpc.broadcast).toHaveBeenCalledWith({}, { targets: ["blk-a"], message: "hi" });
        expect(result.failed).toEqual([{ id: "blk-gone", error: "can't message: no longer listed" }]);
    });
});

describe("stop on a mixed selection", () => {
    it("stops what it can reach and reports the rest, and keeps the staged plan", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", LOAP, MANOZ], true);
        const staged = { batch_size: 2, max_fail_percentage: 50 };

        const result = await vm.bulkStopSelection({ staged });

        expect(rpc.stop).toHaveBeenCalledWith({}, { targets: ["blk-a", LOAP], staged });
        expect(result.failed.map((f) => f.id)).toEqual([MANOZ]);
        expect(result.failed[0].error).toMatch(/can't stop/);
    });

    it("does not call the backend for a selection nothing can reach, and clears it", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([MANOZ, OPAZ], true);
        await vm.bulkStopSelection();
        expect(rpc.stop).not.toHaveBeenCalled();
        expect(vm.selectedBlockIdsAtom().size).toBe(0);
    });
});

describe("selection", () => {
    it("a machine's agents can be added and removed together", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([MANOZ, OPAZ], true);
        expect([...vm.selectedBlockIdsAtom()]).toEqual([MANOZ, OPAZ]);
        vm.setManySelected([MANOZ, OPAZ], false);
        expect(vm.selectedBlockIdsAtom().size).toBe(0);
    });

    // The view calls pruneSelection with its tree's keys whenever its list
    // changes (docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §2).
    it("drops a remote agent that has gone away, and keeps the rest", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", LOAP, MANOZ, OPAZ], true);
        const lan = structuredClone(both);
        lan.hosts[1].channels[0].agents = [{ name: "Opaz", block_id: "" }]; // Manoz left
        rpc.other.mockResolvedValue(lan);
        await vm.loadOtherInstances();
        vm.pruneSelection(LOCAL);
        expect([...vm.selectedBlockIdsAtom()].sort()).toEqual(["blk-a", LOAP, OPAZ].sort());
    });

    it("drops the agents of a section that went stale", async () => {
        const vm = await modelWith(both);
        vm.setManySelected([MANOZ, OPAZ, LOAP], true);
        const stale = structuredClone(both);
        stale.hosts[1].channels[0].stale = true;
        rpc.other.mockResolvedValue(stale);
        await vm.loadOtherInstances();
        vm.pruneSelection(LOCAL);
        expect([...vm.selectedBlockIdsAtom()]).toEqual([LOAP]);
    });

    it("drops a local agent that left the tree: the '2 checked, send to 4' bug", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", "blk-b", MANOZ], true);
        vm.pruneSelection(["blk-a"]); // blk-b's pane closed
        expect([...vm.selectedBlockIdsAtom()].sort()).toEqual(["blk-a", MANOZ].sort());
    });

    it("keeps the same selection object when nothing needs dropping", async () => {
        const vm = await modelWith(both);
        vm.setManySelected(["blk-a", LOAP], true);
        const before = vm.selectedBlockIdsAtom();
        vm.pruneSelection(LOCAL);
        expect(vm.selectedBlockIdsAtom()).toBe(before);
    });
});
