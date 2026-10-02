// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { remoteSections, seenAgo, type RemoteHost, type SwarmOtherInstances } from "./swarm-remote";

const agent = (name: string) => ({ name, block_id: `blk-${name}` });
const channel = (name: string, agents: string[], stale = false) => ({
    channel: name,
    seen_at_ms: 1000,
    stale,
    agents: agents.map(agent),
});
const data = (hosts: RemoteHost[]): SwarmOtherInstances => ({ hostname: "narko", channel: "stable", hosts });

describe("remoteSections", () => {
    it("names this machine's other channels, since it always has its own too", () => {
        const s = remoteSections(
            data([{ host_id: "host:narko", display_name: "narko", tier: "host", channels: [channel("dev-fix-lan", ["Loap"])] }])
        );
        expect(s.map((x) => [x.title, x.badge])).toEqual([["narko · dev-fix-lan", "this machine"]]);
    });

    it("names a LAN host alone when it has one channel, and with the channel when it has two", () => {
        const s = remoteSections(
            data([
                { host_id: "lan:area54", display_name: "Area54", tier: "lan", channels: [channel("stable", ["Manoz"])] },
                {
                    host_id: "lan:starpower",
                    display_name: "starpower",
                    tier: "lan",
                    channels: [channel("stable", ["Opaz"]), channel("dev", ["Korp"])],
                },
            ])
        );
        expect(s.map((x) => x.title)).toEqual(["Area54", "starpower · dev", "starpower · stable"]);
    });

    it("orders this machine, then LAN, then cloud, and sorts agents by name", () => {
        const s = remoteSections(
            data([
                { host_id: "cloud:x", display_name: "home", tier: "cloud", channels: [channel("stable", ["b", "A"])] },
                { host_id: "lan:a", display_name: "Area54", tier: "lan", channels: [channel("stable", ["Manoz"])] },
                { host_id: "host:narko", display_name: "narko", tier: "host", channels: [channel("dev", ["Loap"])] },
            ])
        );
        expect(s.map((x) => x.badge)).toEqual(["this machine", "LAN", "cloud"]);
        expect(s[2].agents.map((a) => a.name)).toEqual(["A", "b"]);
    });

    it("leaves out a channel with no agents, and has nothing for no data", () => {
        expect(
            remoteSections(data([{ host_id: "host:narko", display_name: "narko", tier: "host", channels: [channel("dev", [])] }]))
        ).toEqual([]);
        expect(remoteSections(null)).toEqual([]);
        expect(remoteSections(data([]))).toEqual([]);
    });

    it("keeps keys stable and carries staleness", () => {
        const s = remoteSections(
            data([{ host_id: "host:narko", display_name: "narko", tier: "host", channels: [channel("dev", ["Loap"], true)] }])
        );
        expect(s[0].key).toBe("host:narko/dev");
        expect(s[0].stale).toBe(true);
    });
});

describe("seenAgo", () => {
    it("reads seconds, minutes and hours", () => {
        expect(seenAgo(0, 45_000)).toBe("seen 45s ago");
        expect(seenAgo(0, 5 * 60_000)).toBe("seen 5m ago");
        expect(seenAgo(0, 3 * 3600_000)).toBe("seen 3h ago");
    });
});
