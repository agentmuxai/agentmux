// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import {
    hostsWithSeveralInstances,
    lastSeenText,
    middleTruncate,
    osAndChannel,
    peerKeyRefusals,
    showsChannel,
} from "./lan-peer";

const peer = (agents: string[]): LanInstance => ({ agents }) as LanInstance;
const pin = (agent_id: string, refused?: { key_fp: string; last_seen_at: number; count: number }): LanPinInfo => ({
    agent_id,
    key_fp: "aa:bb",
    pinned_at: 1,
    uid_pins: 0,
    refused,
});

describe("peerKeyRefusals", () => {
    it("reports a refusal for an agent the peer hosts, whatever the case of the name", () => {
        const found = peerKeyRefusals(peer(["Ann"]), [pin("ann", { key_fp: "x", last_seen_at: 9, count: 2 })]);
        expect(found.map((f) => [f.agent, f.refused.count])).toEqual([["ann", 2]]);
    });

    it("ignores a pin that was never refused and a refusal for an agent on another peer", () => {
        const pins = [pin("ann"), pin("bob", { key_fp: "x", last_seen_at: 9, count: 1 })];
        expect(peerKeyRefusals(peer(["ann"]), pins)).toEqual([]);
    });

    it("lists the newest refusal first", () => {
        const pins = [
            pin("ann", { key_fp: "x", last_seen_at: 5, count: 1 }),
            pin("bob", { key_fp: "y", last_seen_at: 8, count: 1 }),
        ];
        expect(peerKeyRefusals(peer(["ann", "bob"]), pins).map((f) => f.agent)).toEqual(["bob", "ann"]);
    });
});

describe("lastSeenText", () => {
    it("steps from seconds to days", () => {
        expect(lastSeenText(100, 102)).toBe("just now");
        expect(lastSeenText(100, 142)).toBe("42 s ago");
        expect(lastSeenText(0, 300)).toBe("5 min ago");
        expect(lastSeenText(0, 3 * 3600)).toBe("3 h ago");
        expect(lastSeenText(0, 2 * 86400)).toBe("2 days ago");
    });

    it("never goes negative when a peer's clock is ahead", () => {
        expect(lastSeenText(200, 100)).toBe("just now");
    });
});

describe("osAndChannel", () => {
    it("names the OS and the channel, leaving out what was not advertised", () => {
        expect(osAndChannel({ os: "macos", channel: "stable" })).toBe("macOS · stable");
        expect(osAndChannel({ os: "", channel: "dev" })).toBe("dev");
        expect(osAndChannel({ os: "linux", channel: "" })).toBe("Linux");
        expect(osAndChannel({ os: "", channel: "" })).toBe("");
    });
});

const on = (hostname: string, channel = "stable") => ({ hostname, channel });

describe("hostsWithSeveralInstances", () => {
    it("lists a host only once two instances of it are listed", () => {
        expect([...hostsWithSeveralInstances([on("charlie")])]).toEqual([]);
        expect([...hostsWithSeveralInstances([on("charlie"), on("area54")])]).toEqual([]);
        expect([...hostsWithSeveralInstances([on("charlie"), on("charlie", "local-3948424")])]).toEqual(["charlie"]);
    });

    it("counts three channels on one host and keeps the other hosts out", () => {
        const peers = [on("charlie"), on("charlie", "dev"), on("charlie", "local-1"), on("area54")];
        expect([...hostsWithSeveralInstances(peers)]).toEqual(["charlie"]);
    });

    it("groups hostnames without regard to case or edge spaces", () => {
        expect([...hostsWithSeveralInstances([on("Charlie"), on("charlie "), on("CHARLIE", "dev")])]).toEqual(["charlie"]);
    });

    it("counts this machine's own instance, which is not a row", () => {
        expect([...hostsWithSeveralInstances([on("narko", "local-2")], "NARKO")]).toEqual(["narko"]);
        expect([...hostsWithSeveralInstances([on("charlie")], "narko")]).toEqual([]);
    });

    it("ignores a peer that has no hostname yet", () => {
        expect([...hostsWithSeveralInstances([on(""), on("")])]).toEqual([]);
    });
});

describe("showsChannel", () => {
    const shared = new Set(["charlie"]);
    it("names the channel only on a shared host, and only when the peer advertised one", () => {
        expect(showsChannel(on("Charlie", "dev"), shared)).toBe(true);
        expect(showsChannel(on("charlie", ""), shared)).toBe(false);
        expect(showsChannel(on("area54", "stable"), shared)).toBe(false);
    });
});

describe("middleTruncate", () => {
    it("cuts a long name in the middle and leaves a short one", () => {
        expect(middleTruncate("stable", 18)).toBe("stable");
        const cut = middleTruncate("local-3948424-feature-branch", 18);
        expect(cut).toHaveLength(18);
        expect(cut.startsWith("local-39")).toBe(true);
        expect(cut.endsWith("branch")).toBe(true);
        expect(cut).toContain("…");
    });
});
