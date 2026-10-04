// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the fleet toolbar knows about each thing that can be selected.
 *
 * A selection is a set of strings. A target this instance or another channel on
 * this machine runs is its block id, which the fleet RPCs already take. A target
 * with no block id here (an agent on a LAN peer or in the cloud) is keyed by
 * where it is and its name. Which actions can reach a target depends on how it
 * was found: another channel on this machine is reached over loopback, while an
 * agent on another machine can't be acted on yet because nothing there can
 * verify the request is yours.
 * docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §4, §5, §6.
 */

import type { RemoteAgent, RemoteSection } from "./swarm-remote";

export type FleetAction = "broadcast" | "stop";

export interface FleetTargetInfo {
    key: string;
    name: string;
    /** The section's title ("narko · dev-fix-lan"); null for this instance. */
    where: string | null;
    /** How it was found ("this machine", "LAN", "cloud"); null for this instance. */
    badge: string | null;
    platform: string | null;
    remote: boolean;
    stale: boolean;
    /** Whether the target has a block id an action can name. */
    actionable: boolean;
}

const NO_VERIFIED_LINK = "no verified link to other machines yet";

/** Why `action` can't reach `target`, or null when it can. */
export function unavailableReason(target: FleetTargetInfo, action: FleetAction): string | null {
    if (target.actionable) return null;
    return action === "stop" ? `can't stop: ${NO_VERIFIED_LINK}` : `can't message: ${NO_VERIFIED_LINK}`;
}

/** The selection key of a remote agent: its block id when it has one (this
 *  machine's other channels), else where it is and its name. */
export function remoteAgentKey(section: Pick<RemoteSection, "key">, agent: RemoteAgent): string {
    return agent.block_id ? agent.block_id : `remote:${section.key}/${agent.name}`;
}

/** Every remote agent as a target, by selection key. A stale section's agents
 *  are listed but flagged, because they are about to disappear. */
export function remoteFleetTargets(sections: RemoteSection[]): Map<string, FleetTargetInfo> {
    const out = new Map<string, FleetTargetInfo>();
    for (const section of sections) {
        for (const agent of section.agents) {
            const key = remoteAgentKey(section, agent);
            out.set(key, {
                key,
                name: agent.name,
                where: section.title,
                badge: section.badge,
                platform: section.platform,
                remote: true,
                stale: section.stale,
                // Another channel on this machine is reached over loopback. Anything
                // else has no verified path (spec §6).
                actionable: section.tier === "host" && !!agent.block_id,
            });
        }
    }
    return out;
}

/** This instance's own agents, by block id. */
export function localFleetTargets(nodes: { blockId?: string | null; agentName: string }[]): Map<string, FleetTargetInfo> {
    const out = new Map<string, FleetTargetInfo>();
    for (const n of nodes) {
        if (!n.blockId) continue;
        out.set(n.blockId, {
            key: n.blockId,
            name: n.agentName,
            where: null,
            badge: null,
            platform: null,
            remote: false,
            stale: false,
            actionable: true,
        });
    }
    return out;
}

/** The keys an action can name, and the targets it can't reach with why. A key
 *  that is neither local nor remote any more (the agent went away) is passed
 *  through as before: the action reports it per target. */
export function partitionTargets(
    keys: string[],
    action: FleetAction,
    known: Map<string, FleetTargetInfo>
): { reachable: string[]; unreachable: { key: string; reason: string }[] } {
    const reachable: string[] = [];
    const unreachable: { key: string; reason: string }[] = [];
    for (const key of keys) {
        const target = known.get(key);
        const reason = target ? unavailableReason(target, action) : null;
        if (reason) unreachable.push({ key, reason });
        else reachable.push(key);
    }
    return { reachable, unreachable };
}

/** "Loap · narko · dev-fix-lan", or just "Loap" for this instance's own. Falls
 *  back to the key for something no longer listed. */
export function targetLabel(key: string, known: Map<string, FleetTargetInfo>): string {
    const t = known.get(key);
    if (!t) return key;
    return t.where ? `${t.name} · ${t.where}` : t.name;
}

/** How many of `keys` are agents on other instances. */
export function remoteCount(keys: Iterable<string>, known: Map<string, FleetTargetInfo>): number {
    let n = 0;
    for (const k of keys) if (known.get(k)?.remote) n++;
    return n;
}
