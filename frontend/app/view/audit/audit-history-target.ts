// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the History reader needs that an Audit link doesn't carry: the agent's provider (its transcript's
// format) and, for a link that has only a time, the conversation that time falls in.

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AuditHistoryLink } from "@/types/rpc/AuditHistoryLink";

export interface HistoryTarget {
    agentId: string;
    agentName: string;
    provider?: string;
    /** An archive's zone id; absent for the current conversation. */
    zone?: string;
    /** False when the conversation holding the time can't be told: the reader opens at its end, not somewhere wrong. */
    positioned: boolean;
}

/** The most archives one listing gives (newest first); an agent with more has older ones the listing leaves out. */
export const ARCHIVE_LIST_CAP = 100;

/** Whether the listing reaches back to `atMs`: a list cut at its cap may have left out the archive holding it. */
export function archiveKnown(archives: { archived_at_ms: number }[], atMs: number): boolean {
    return archives.length < ARCHIVE_LIST_CAP || atMs >= Math.min(...archives.map((a) => a.archived_at_ms));
}

/**
 * The first conversation archived at or after `atMs` holds what was said then; none means the current one
 * (the same rule srv uses for a start or end's byte, `audit_query::segments::zone_at`).
 */
export function archiveAt(archives: { archive_zoneid: string; archived_at_ms: number }[], atMs: number): string | undefined {
    return archives.filter((a) => a.archived_at_ms >= atMs).sort((a, b) => a.archived_at_ms - b.archived_at_ms)[0]?.archive_zoneid;
}

export async function resolveHistoryTarget(link: AuditHistoryLink): Promise<HistoryTarget> {
    const agentId = link.definition_id ?? link.agent_id;
    const target: HistoryTarget = { agentId, agentName: link.agent_id, positioned: true, ...(link.zone ? { zone: link.zone } : {}) };
    if (!link.definition_id) return target;
    const [agents, archives] = await Promise.all([
        RpcApi.HistoryAgentsCommand(TabRpcClient, {}).catch(() => null),
        link.zone ? Promise.resolve(null) : RpcApi.AgentSessionListArchivesCommand(TabRpcClient, { definition_id: agentId, limit: ARCHIVE_LIST_CAP }).catch(() => null),
    ]);
    const known = agents?.agents.find((a) => a.definition_id === agentId);
    if (known?.provider) target.provider = known.provider;
    if (archives && !archiveKnown(archives, link.at_ms)) target.positioned = false;
    const zone = archives && target.positioned ? archiveAt(archives, link.at_ms) : undefined;
    if (zone) target.zone = zone;
    return target;
}
