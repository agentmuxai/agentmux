// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's sections for other AgentMux instances: one per host and channel,
 * below this instance's own tree.
 * docs/specs/SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md section 3.
 *
 * The data comes from `swarm.other-instances` (crates/srv/src/backend/swarm_remote.rs).
 * Phase 1 is this machine's other channels, names only; LAN hosts and cloud
 * installs arrive later as more hosts with their own `tier`.
 */

export interface RemoteAgent {
    name: string;
    block_id: string;
}

export interface RemoteChannel {
    channel: string;
    seen_at_ms: number;
    stale: boolean;
    agents: RemoteAgent[];
}

export interface RemoteHost {
    host_id: string;
    display_name: string;
    tier: "host" | "lan" | "cloud";
    channels: RemoteChannel[];
}

export interface SwarmOtherInstances {
    hostname: string;
    channel: string;
    hosts: RemoteHost[];
}

/** One section the view renders. */
export interface RemoteSection {
    /** Stable across refreshes: the collapse state is keyed on it. */
    key: string;
    title: string;
    /** How it was found, shown as a badge: trust and freshness differ. */
    badge: string;
    stale: boolean;
    seenAtMs: number;
    agents: RemoteAgent[];
}

const BADGES: Record<RemoteHost["tier"], string> = { host: "this machine", lan: "LAN", cloud: "cloud" };
const TIER_ORDER: Record<RemoteHost["tier"], number> = { host: 0, lan: 1, cloud: 2 };

/**
 * Sections in display order, with the owner's naming rule: a host is named alone
 * when it has one channel, and "host · channel" when it has more than one in
 * total. This machine always counts its own channel, so its other channels are
 * always named. Sections with no agents are left out.
 */
export function remoteSections(data: SwarmOtherInstances | null | undefined): RemoteSection[] {
    if (!data) return [];
    const hosts = [...data.hosts].sort(
        (a, b) => TIER_ORDER[a.tier] - TIER_ORDER[b.tier] || a.display_name.localeCompare(b.display_name)
    );
    const out: RemoteSection[] = [];
    for (const host of hosts) {
        const channelsOnHost = host.channels.length + (host.tier === "host" ? 1 : 0);
        for (const ch of [...host.channels].sort((a, b) => a.channel.localeCompare(b.channel))) {
            if (ch.agents.length === 0) continue;
            out.push({
                key: `${host.host_id}/${ch.channel}`,
                title: channelsOnHost > 1 ? `${host.display_name} · ${ch.channel}` : host.display_name,
                badge: BADGES[host.tier] ?? host.tier,
                stale: ch.stale,
                seenAtMs: ch.seen_at_ms,
                agents: [...ch.agents].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" })),
            });
        }
    }
    return out;
}

/** "seen 2m ago" for a stale section, from Unix ms. */
export function seenAgo(seenAtMs: number, nowMs: number): string {
    const s = Math.max(0, Math.round((nowMs - seenAtMs) / 1000));
    if (s < 60) return `seen ${s}s ago`;
    const m = Math.round(s / 60);
    if (m < 60) return `seen ${m}m ago`;
    return `seen ${Math.round(m / 60)}h ago`;
}
