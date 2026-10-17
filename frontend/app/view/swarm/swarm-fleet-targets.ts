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

import type { FleetActionResult } from "@/app/store/rpc-api";
import type { RemoteAgent, RemoteSection } from "./swarm-remote";

export type FleetAction = "broadcast" | "stop" | "supervise";

export interface FleetTargetInfo {
    key: string;
    name: string;
    /** The section's title ("narko · dev-fix-lan"); null for this instance. */
    where: string | null;
    /** How it was found ("this machine", "LAN", "cloud"); null for this instance. */
    badge: string | null;
    platform: string | null;
    remote: boolean;
    /** Runs on another machine (LAN or cloud), not another channel of this one. */
    otherMachine: boolean;
    stale: boolean;
    /** Whether the target has a block id an action can name. */
    actionable: boolean;
}

const NO_VERIFIED_LINK = "no verified link to other machines yet";
const NO_LONGER_LISTED = "no longer listed";
const NOT_SEEN_RECENTLY = "not seen recently";

const VERBS: Record<FleetAction, string> = { broadcast: "can't message", stop: "can't stop", supervise: "can't supervise" };

/** Why `action` can't reach `target`, or null when it can. The Supervisor runs
 *  in each AgentMux for its own agents, so it reaches this instance's only
 *  (docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §2.1). A stale target is
 *  about to disappear and its last state may be long out of date. */
export function unavailableReason(target: FleetTargetInfo, action: FleetAction): string | null {
    if (action === "supervise") return target.remote ? "can't supervise: only this AgentMux's own agents" : null;
    if (target.stale) return `${VERBS[action]}: ${NOT_SEEN_RECENTLY}`;
    if (target.actionable) return null;
    return `${VERBS[action]}: ${NO_VERIFIED_LINK}`;
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
                otherMachine: section.tier !== "host",
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
        if (n.blockId) out.set(n.blockId, localTarget(n.blockId, n.agentName));
    }
    return out;
}

/** The keys an action can name, and the targets it can't reach with why. A key
 *  the Swarm no longer lists (its agent went away) can't be reached either: the
 *  view prunes those from the selection, so this only catches a race, and it
 *  says so rather than sending it on
 *  (docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §2.2). */
export function partitionTargets(
    keys: string[],
    action: FleetAction,
    known: Map<string, FleetTargetInfo>
): { reachable: string[]; unreachable: { key: string; reason: string }[] } {
    const reachable: string[] = [];
    const unreachable: { key: string; reason: string }[] = [];
    for (const key of keys) {
        const target = known.get(key);
        const reason = target ? unavailableReason(target, action) : `${VERBS[action]}: ${NO_LONGER_LISTED}`;
        if (reason) unreachable.push({ key, reason });
        else reachable.push(key);
    }
    return { reachable, unreachable };
}

/** "2 of 4 can't be messaged: no verified link to other machines yet", or null
 *  when every selected target can be reached. Each distinct reason once, so the
 *  toolbar says why its two numbers differ (spec §2.2). */
export function gapSummary(unreachable: { reason: string }[], total: number, action: FleetAction): string | null {
    if (unreachable.length === 0) return null;
    const details = [...new Set(unreachable.map((u) => u.reason.replace(/^[^:]*:\s*/, "")))];
    const verb = action === "broadcast" ? "can't be messaged" : action === "stop" ? "can't be stopped" : "can't be supervised";
    return `${unreachable.length} of ${total} ${verb}: ${details.join("; ")}`;
}

/** This instance's agent `key` as a target, for a list that only has its key. */
export function localTarget(key: string, name = key): FleetTargetInfo {
    return { key, name, where: null, badge: null, platform: null, remote: false, otherMachine: false, stale: false, actionable: true };
}

/** "Loap · narko · dev-fix-lan", or just "Loap" for this instance's own. Falls
 *  back to the key for something no longer listed. */
export function targetLabel(key: string, known: Map<string, FleetTargetInfo>): string {
    const t = known.get(key);
    if (!t) return key;
    return t.where ? `${t.name} · ${t.where}` : t.name;
}

/** The last broadcast or stop, with a name for every target so the result
 *  panel can say which agent on which machine, not a block id. */
export interface FleetResultEntry {
    action: "broadcast" | "bulk-stop" | "supervisor";
    result: FleetActionResult;
    labels: Record<string, string>;
}

export const EMPTY_FLEET_RESULT: FleetActionResult = { succeeded: [], failed: [], aborted_early: false };

/** `result` plus the targets the action could not reach, each with its reason,
 *  so a target is never silently left out. */
export function withUnreachable(result: FleetActionResult, unreachable: { key: string; reason: string }[]): FleetActionResult {
    if (unreachable.length === 0) return result;
    return { ...result, failed: [...result.failed, ...unreachable.map((u) => ({ id: u.key, error: u.reason }))] };
}

export function labelsOf(keys: string[], known: Map<string, FleetTargetInfo>): Record<string, string> {
    return Object.fromEntries(keys.map((key) => [key, targetLabel(key, known)]));
}
