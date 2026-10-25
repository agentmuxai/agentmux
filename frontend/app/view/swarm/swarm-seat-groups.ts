// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's "Group by lead": its rows in groups by whom each agent reports
 * to (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4). The operator
 * heads the agents that report to it, leaders among them; each agent's
 * reports follow it, one step in; a lead with no row here heads its own
 * group, its name as the heading; agents with no seat go under "No seat", and other instances' agents
 * with no seat under their own heading. An other instance's agent that has a seat is grouped like any
 * other, by its lead's name within its own section (swarm-seats.tsx gives it the keys). Within a group
 * the rows keep the order they are given (swarm-order.ts).
 */

import { leadName, readSeatMeta, SEAT_META_KEY } from "@/app/view/agent/seat-format";
import type { SeatSummary } from "@/types/rpc/SeatSummary";
import type { RemoteRow } from "./swarm-remote";

export type SwarmGroupBy = "none" | "lead";

/** The Swarm pane's meta key for the choice, kept with the pane like its zoom. */
export const GROUP_BY_META_KEY = "swarm:groupby";

export function readGroupBy(meta: Record<string, unknown> | undefined): SwarmGroupBy {
    return meta?.[GROUP_BY_META_KEY] === "lead" ? "lead" : "none";
}

/** What grouping needs to know about one row. */
export interface SeatRowInfo {
    /** The agent's uid (block meta `agentId`); null when the pane names none. */
    agentId: string | null;
    seat: SeatSummary | null;
    /** An agent of another instance. One with a `seat` is grouped by it; one without goes under its own heading. */
    remote: boolean;
}

/** Another instance's agents have no uid here, so within a section a name stands for one. */
const remoteAgentId = (sectionKey: string, name: string) => `remote:${sectionKey}/${name.toLowerCase()}`;

/**
 * What grouping knows about an agent of another instance: its seat as it
 * reports it (an unknown role is dropped), with the agent and its lead named
 * by section and name, so a remote worker follows a remote lead of the same
 * section. A lead the peer couldn't name stays as it was: its group is
 * headed "an agent on another machine".
 */
export function remoteSeatInfo(row: RemoteRow): SeatRowInfo {
    const seat = readSeatMeta({ [SEAT_META_KEY]: row.agent.seat });
    const lead = seat?.lead;
    const named = seat && lead?.name ? { ...seat, lead: { id: remoteAgentId(row.section.key, lead.name), name: lead.name } } : seat;
    return { agentId: remoteAgentId(row.section.key, row.agent.name), seat: named, remote: true };
}

/** How deep a row sits under its group's heading: its lead's depth plus one, cut here. */
const MAX_SEAT_DEPTH = 3;

export type SeatListEntry =
    | { kind: "heading"; id: string; label: string; title: string; count: number }
    | { kind: "row"; key: string; depth: number };

const OPERATOR_HEADING = "The operator";
const NO_SEAT_HEADING = "No seat";
const REMOTE_HEADING = "Other instances";

/** The rows in `order`, grouped. A key `info` doesn't describe goes under "No seat". */
export function groupRowsByLead(order: readonly string[], info: ReadonlyMap<string, SeatRowInfo>): SeatListEntry[] {
    const seated: string[] = [];
    const unseated: string[] = [];
    const remote: string[] = [];
    for (const key of order) {
        const row = info.get(key);
        if (row?.remote && !row.seat) remote.push(key);
        else if (row?.seat) seated.push(key);
        else unseated.push(key);
    }

    // The agents shown here, by uid, and each agent's reports.
    const shown = new Set<string>();
    for (const key of seated) {
        const id = info.get(key)?.agentId;
        if (id) shown.add(id);
    }
    const reportsOf = new Map<string, string[]>();
    const toOperator: string[] = [];
    const toAbsentLead = new Map<string, string[]>();
    for (const key of seated) {
        const lead = info.get(key)!.seat!.lead;
        if (!lead) toOperator.push(key);
        else if (shown.has(lead.id)) reportsOf.set(lead.id, [...(reportsOf.get(lead.id) ?? []), key]);
        else toAbsentLead.set(lead.id, [...(toAbsentLead.get(lead.id) ?? []), key]);
    }

    const placed = new Set<string>();
    // An agent shown in two panes heads its reports once, at its first row.
    const headed = new Set<string>();
    const subtree = (key: string, depth: number): SeatListEntry[] => {
        if (placed.has(key)) return [];
        placed.add(key);
        const out: SeatListEntry[] = [{ kind: "row", key, depth: Math.min(depth, MAX_SEAT_DEPTH) }];
        const id = info.get(key)?.agentId;
        if (id && !headed.has(id)) {
            headed.add(id);
            for (const child of reportsOf.get(id) ?? []) out.push(...subtree(child, depth + 1));
        }
        return out;
    };

    const entries: SeatListEntry[] = [];
    const group = (id: string, label: string, title: string, rows: SeatListEntry[]) => {
        const count = rows.filter((r) => r.kind === "row").length;
        if (count > 0) entries.push({ kind: "heading", id, label, title, count }, ...rows);
    };

    const operatorRows = toOperator.flatMap((key) => subtree(key, 0));
    const absentGroups = [...toAbsentLead.entries()].map(([leadId, keys]) => {
        const lead = info.get(keys[0])!.seat!.lead;
        return { leadId, label: leadName(lead), rows: keys.flatMap((key) => subtree(key, 0)) };
    });
    // srv refuses a cycle (§2), but a seat changing between two reads can show one
    // for a moment: anything still unplaced joins the operator's group, not lost.
    for (const key of seated) operatorRows.push(...subtree(key, 0));

    group(
        "operator",
        OPERATOR_HEADING,
        "Agents that report to the operator, each followed by its reports",
        operatorRows
    );
    for (const g of absentGroups) {
        group(`lead:${g.leadId}`, g.label, `Agents that report to ${g.label}, who has no pane open here`, g.rows);
    }
    group(
        "none",
        NO_SEAT_HEADING,
        "Agents with no seat set",
        unseated.map((key) => ({ kind: "row", key, depth: 0 }))
    );
    group(
        "remote",
        REMOTE_HEADING,
        "Agents of other AgentMux instances that share no seat",
        remote.map((key) => ({ kind: "row", key, depth: 0 }))
    );
    return entries;
}
