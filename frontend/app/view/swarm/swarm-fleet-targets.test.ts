// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    localFleetTargets,
    partitionTargets,
    remoteAgentKey,
    remoteCount,
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
            stale: false,
        });
        expect(remote.get(MANOZ)).toMatchObject({ where: "Area54", badge: "LAN", platform: "macOS" });
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
    it("says why, per action, for an agent on another machine", () => {
        const manoz = remote.get(MANOZ)!;
        expect(unavailableReason(manoz, "stop")).toMatch(/^can't stop: .*verified link/);
        expect(unavailableReason(manoz, "broadcast")).toMatch(/^can't message: .*verified link/);
    });

    it("is null for anything an action can reach", () => {
        expect(unavailableReason(remote.get("blk-loap")!, "stop")).toBeNull();
        expect(unavailableReason(remote.get("blk-loap")!, "broadcast")).toBeNull();
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

    it("passes through a key no longer listed: the action reports it per target", () => {
        expect(partitionTargets(["gone-block"], "broadcast", known).reachable).toEqual(["gone-block"]);
    });

    it("keeps the order of the selection", () => {
        expect(partitionTargets(["blk-loap", "blk-korp"], "broadcast", known).reachable).toEqual(["blk-loap", "blk-korp"]);
    });
});

describe("targetLabel and remoteCount", () => {
    const known = new Map([...localFleetTargets([{ blockId: "blk-korp", agentName: "Korp" }]), ...remote]);

    it("names an agent here by name alone, and one elsewhere with its machine", () => {
        expect(targetLabel("blk-korp", known)).toBe("Korp");
        expect(targetLabel("blk-loap", known)).toBe("Loap · narko · dev-fix-lan");
    });

    it("falls back to the key for something no longer listed", () => {
        expect(targetLabel("blk-gone", known)).toBe("blk-gone");
    });

    it("counts only agents on other instances", () => {
        expect(remoteCount(["blk-korp", "blk-loap", "remote:lan:area54#1/stable/Opaz", "blk-gone"], known)).toBe(2);
    });
});
