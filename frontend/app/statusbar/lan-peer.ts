// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the network pill's peer rows say about a peer, apart from drawing it.

import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import type { RefusedKeyInfo } from "@/types/rpc/RefusedKeyInfo";

export interface PeerKeyRefusal {
    agent: string;
    refused: RefusedKeyInfo;
}

/**
 * The agents of `peer` for whose name a message was refused because it came with a different key
 * than the one pinned. A pin is by name, and a peer lists the names it hosts, so the match is the
 * lower-cased name. Newest refusal first.
 */
export function peerKeyRefusals(peer: LanInstance, pins: readonly LanPinInfo[]): PeerKeyRefusal[] {
    const hosted = new Set(peer.agents.map((a) => a.toLowerCase()));
    const out: PeerKeyRefusal[] = [];
    for (const pin of pins) {
        if (pin.refused && hosted.has(pin.agent_id.toLowerCase())) {
            out.push({ agent: pin.agent_id, refused: pin.refused });
        }
    }
    return out.sort((a, b) => b.refused.last_seen_at - a.refused.last_seen_at);
}

/** "just now", "42 s ago", "5 min ago", "3 h ago", "2 days ago" for a time in epoch seconds. */
export function lastSeenText(seenSecs: number, nowSecs: number): string {
    const s = Math.max(0, Math.round(nowSecs - seenSecs));
    if (s < 5) return "just now";
    if (s < 60) return `${s} s ago`;
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    if (s < 86400) return `${Math.round(s / 3600)} h ago`;
    return `${Math.round(s / 86400)} days ago`;
}

const OS_LABELS: Record<string, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

/** "Windows · stable": the peer's OS and channel, leaving out what it did not advertise. */
export function osAndChannel(peer: Pick<LanInstance, "os" | "channel">): string {
    const os = OS_LABELS[peer.os] ?? peer.os;
    return [os, peer.channel].filter((p) => p).join(" · ");
}
