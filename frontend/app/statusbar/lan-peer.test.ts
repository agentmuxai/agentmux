// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import { lastSeenText, osAndChannel, peerKeyRefusals } from "./lan-peer";

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
