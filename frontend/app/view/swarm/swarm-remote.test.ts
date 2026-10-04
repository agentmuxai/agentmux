// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { platformLabel, remoteSections, seenAgo, type RemoteHost, type SwarmOtherInstances } from "./swarm-remote";

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

// SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md section 3.
describe("platformLabel", () => {
    it("names the three platforms an instance advertises", () => {
        expect(platformLabel("windows")).toBe("Windows");
        expect(platformLabel("macos")).toBe("macOS");
        expect(platformLabel("linux")).toBe("Linux");
    });

    it("gives no tag for anything else: unknown, missing, or text a peer made up", () => {
        for (const v of [undefined, null, "", "freebsd", "Windows", "<b>linux</b>", "constructor", "__proto__", "toString"]) {
            expect(platformLabel(v as string | null | undefined)).toBeNull();
        }
    });
});

describe("remoteSections: platform", () => {
    const withOs = (os: string | undefined) => ({ ...channel("stable", ["Manoz"]), os });

    it("carries each section's own platform, so two instances on one host keep theirs", () => {
        const s = remoteSections(
            data([
                {
                    host_id: "lan:narko",
                    display_name: "narko",
                    tier: "lan",
                    channels: [{ ...withOs("windows"), channel: "stable" }, { ...withOs("linux"), channel: "wsl" }],
                },
                { host_id: "lan:a", display_name: "Area54", tier: "lan", channels: [withOs("macos")] },
            ])
        );
        expect(s.map((x) => [x.title, x.platform])).toEqual([
            ["Area54", "macOS"],
            ["narko · stable", "Windows"],
            ["narko · wsl", "Linux"],
        ]);
    });

    it("has no platform for a peer that did not advertise one", () => {
        const s = remoteSections(data([{ host_id: "lan:old", display_name: "old", tier: "lan", channels: [withOs(undefined)] }]));
        expect(s[0].platform).toBeNull();
    });

    it("carries how the section was found", () => {
        const s = remoteSections(data([{ host_id: "cloud:x", display_name: "home", tier: "cloud", channels: [withOs("linux")] }]));
        expect([s[0].tier, s[0].badge]).toEqual(["cloud", "cloud"]);
    });
});
