// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    gapSummary,
    localFleetTargets,
    partitionTargets,
    remoteAgentKey,
    remoteFleetTargets,
    targetLabel,
    unavailableReason,
} from "./swarm-fleet-targets";
import { remoteSections, type SwarmOtherInstances } from "./swarm-remote";

const data: SwarmOtherInstances = {
    hostname: "narko",
    channel: "stable",
    hosts: [
        {
            host_id: "host:narko",
            display_name: "narko",
            tier: "host",
            channels: [{ channel: "dev-fix-lan", seen_at_ms: 1, stale: false, os: "windows", agents: [{ name: "Loap", block_id: "blk-loap" }] }],
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
            host_id: "lan:old#2",
            display_name: "oldbox",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: 1, stale: true, agents: [{ name: "Gone", block_id: "" }] }],
        },
    ],
};
const remote = remoteFleetTargets(remoteSections(data));
const MANOZ = "remote:lan:area54#1/stable/Manoz";

describe("remoteAgentKey", () => {
    it("is the block id when the agent has one, so the fleet RPCs can name it", () => {
        expect(remoteAgentKey({ key: "host:narko/dev-fix-lan" }, { name: "Loap", block_id: "blk-loap" })).toBe("blk-loap");
    });

    it("is where it is and its name when it has none, and differs between machines", () => {
        const a = remoteAgentKey({ key: "lan:a#1/stable" }, { name: "Agent3", block_id: "" });
        const b = remoteAgentKey({ key: "lan:b#2/stable" }, { name: "Agent3", block_id: "" });
        expect(a).toBe("remote:lan:a#1/stable/Agent3");
        expect(a).not.toBe(b);
    });
});

describe("remoteFleetTargets", () => {
    it("describes each remote agent with its machine, platform and how it was found", () => {
        expect(remote.get("blk-loap")).toMatchObject({
            name: "Loap",
            where: "narko · dev-fix-lan",
            badge: "this machine",
            platform: "Windows",
            remote: true,
            otherMachine: false,
            stale: false,
        });
        expect(remote.get(MANOZ)).toMatchObject({ where: "Area54", badge: "LAN", platform: "macOS", otherMachine: true });
    });

    it("can act on another channel on this machine, and on nothing found over the LAN or cloud", () => {
        expect(remote.get("blk-loap")!.actionable).toBe(true);
        expect(remote.get(MANOZ)!.actionable).toBe(false);
    });

    it("flags a stale section's agents", () => {
        expect(remote.get("remote:lan:old#2/stable/Gone")!.stale).toBe(true);
    });
});

describe("unavailableReason", () => {
    // docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §4.2.
    it("lets a broadcast reach a LAN peer's agent, but not a stop", () => {
        const manoz = remote.get(MANOZ)!;
        expect(unavailableReason(manoz, "broadcast")).toBeNull();
        expect(unavailableReason(manoz, "stop")).toBe("can't stop: stopping an agent on another machine isn't supported yet");
    });

    it("reaches nothing on a cloud install yet, and says why", () => {
        const cloud = remoteFleetTargets(
            remoteSections({
                hostname: "narko",
                channel: "stable",
                hosts: [{ host_id: "cloud:home#3", display_name: "home", tier: "cloud", channels: [{ channel: "stable", seen_at_ms: 1, stale: false, agents: [{ name: "Rook", block_id: "" }] }] }],
            })
        ).get("remote:cloud:home#3/stable/Rook")!;
        expect(unavailableReason(cloud, "broadcast")).toBe("can't message: no verified link to that machine yet");
        expect(unavailableReason(cloud, "stop")).toMatch(/^can't stop/);
    });

    it("is null for anything an action can reach", () => {
        expect(unavailableReason(remote.get("blk-loap")!, "stop")).toBeNull();
        expect(unavailableReason(remote.get("blk-loap")!, "broadcast")).toBeNull();
    });

    // Each AgentMux supervises its own agents (SPEC_SWARM_SUPERVISOR_2026_10_16.md §2.1).
    it("supervises this AgentMux's own agents only, even another channel on this machine", () => {
        const local = localFleetTargets([{ blockId: "blk-korp", agentName: "Korp" }]).get("blk-korp")!;
        expect(unavailableReason(local, "supervise")).toBeNull();
        expect(unavailableReason(remote.get("blk-loap")!, "supervise")).toMatch(/^can't supervise/);
        expect(unavailableReason(remote.get(MANOZ)!, "supervise")).toMatch(/^can't supervise/);
    });
});

describe("partitionTargets", () => {
    const local = localFleetTargets([{ blockId: "blk-korp", agentName: "Korp" }, { blockId: null, agentName: "no block" }]);
    const known = new Map([...local, ...remote]);

    it("names what an action can reach, and lists the rest with the reason instead of dropping them", () => {
        const { reachable, unreachable } = partitionTargets(["blk-korp", "blk-loap", MANOZ], "stop", known);
        expect(reachable).toEqual(["blk-korp", "blk-loap"]);
        expect(unreachable).toEqual([{ key: MANOZ, reason: expect.stringContaining("can't stop") }]);
    });

    // docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §2.2: never sent on.
    it("can't reach a key no longer listed, and says so", () => {
        expect(partitionTargets(["gone-block"], "broadcast", known)).toEqual({
            reachable: [],
            unreachable: [{ key: "gone-block", reason: "can't message: no longer listed" }],
        });
    });

    it("can't reach a stale agent, even on this machine", () => {
        const staleHere = new Map([["blk-x", { ...remote.get("blk-loap")!, key: "blk-x", stale: true }]]);
        expect(partitionTargets(["blk-x"], "stop", staleHere).unreachable).toEqual([{ key: "blk-x", reason: "can't stop: not seen recently" }]);
    });

    it("keeps the order of the selection", () => {
        expect(partitionTargets(["blk-loap", "blk-korp"], "broadcast", known).reachable).toEqual(["blk-loap", "blk-korp"]);
    });
});

describe("gapSummary", () => {
    it("is null when everything selected can be reached", () => {
        expect(gapSummary([], 3, "broadcast")).toBeNull();
    });

    it("says how many of the selection won't be reached, and each reason once", () => {
        const unreachable = [
            { reason: "can't message: no verified link to other machines yet" },
            { reason: "can't message: no verified link to other machines yet" },
            { reason: "can't message: no longer listed" },
        ];
        expect(gapSummary(unreachable, 4, "broadcast")).toBe(
            "3 of 4 can't be messaged: no verified link to other machines yet; no longer listed"
        );
        expect(gapSummary(unreachable.slice(0, 1), 2, "stop")).toBe("1 of 2 can't be stopped: no verified link to other machines yet");
    });
});

describe("targetLabel", () => {
    const known = new Map([...localFleetTargets([{ blockId: "blk-korp", agentName: "Korp" }]), ...remote]);

    it("names an agent here by name alone, and one elsewhere with its machine", () => {
        expect(targetLabel("blk-korp", known)).toBe("Korp");
        expect(targetLabel("blk-loap", known)).toBe("Loap · narko · dev-fix-lan");
    });

    it("falls back to the key for something no longer listed", () => {
        expect(targetLabel("blk-gone", known)).toBe("blk-gone");
    });
});
