// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How an agent's seat reads wherever it is shown: the My Agents tile's role
 * badge and "reports to" line, and the agent pane header's tooltip
 * (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4). One set of words,
 * so a seat reads the same on every surface.
 */

import type { AgentSeat } from "@/types/rpc/AgentSeat";
import type { SeatLead } from "@/types/rpc/SeatLead";
import type { SeatSummary } from "@/types/rpc/SeatSummary";

export type SeatRole = AgentSeat["role"];

/** The roles, in the order the Seat tab offers them, with their labels. */
export const SEAT_ROLES: { value: SeatRole; label: string }[] = [
    { value: "leader", label: "Leader" },
    { value: "worker", label: "Worker" },
    { value: "specialist", label: "Specialist" },
    { value: "platform_tester", label: "Platform tester" },
    { value: "analyst", label: "Analyst" },
    { value: "super", label: "Super agent" },
];

/** srv's limits (`backend::agent_seat`), so the tab stops at the same place. */
export const MAX_QUALIFIER_CHARS = 40;
export const MAX_AREA_CHARS = 120;

/** The block meta key an agent pane holds its agent's seat under (srv `agent_seat::SEAT_META_KEY`). */
export const SEAT_META_KEY = "agent:seat";

export function isSeatRole(role: unknown): role is SeatRole {
    return SEAT_ROLES.some((r) => r.value === role);
}

/** "Platform tester"; a role this build doesn't know shows as written. */
export function roleLabel(role: string): string {
    return SEAT_ROLES.find((r) => r.value === role)?.label ?? role;
}

/** The badge: "Leader · product", or just "Specialist". */
export function seatBadgeText(seat: Pick<SeatSummary, "role" | "qualifier">): string {
    const qualifier = seat.qualifier?.trim();
    return qualifier ? `${roleLabel(seat.role)} · ${qualifier}` : roleLabel(seat.role);
}

/** Who the agent reports to: "the operator", the lead's name, or a lead this machine can't name. */
export function leadName(lead: SeatLead | null | undefined): string {
    if (!lead) return "the operator";
    return lead.name?.trim() || "an agent on another machine";
}

/** The tile's second line: "reports to Camper · panes, layout, tabs". */
export function seatReportsLine(seat: Pick<SeatSummary, "lead" | "area">): string {
    const area = seat.area?.trim();
    const reports = `reports to ${leadName(seat.lead)}`;
    return area ? `${reports} · ${area}` : reports;
}

/** The header tooltip's line: "Leader · product — reports to the operator — agent pane, identity, …". */
export function seatTooltipLine(seat: SeatSummary): string {
    const area = seat.area?.trim();
    const parts = [seatBadgeText(seat), `reports to ${leadName(seat.lead)}`];
    if (area) parts.push(area);
    return parts.join(" — ");
}

const optionalText = (v: unknown): string | null => (typeof v === "string" && v.trim() ? v : null);

/**
 * Each agent's seat as it is shown, by agent uid, from `listagents`: the
 * stored seat, or a spawned worker's effective one. A lead is named from the
 * same list; one not in it keeps a null name, which `leadName` words.
 */
export function seatsByAgent(
    defs: readonly { id: string; name: string; seat?: AgentSeat }[]
): Map<string, SeatSummary> {
    const names = new Map(defs.map((d) => [d.id, d.name]));
    const seats = new Map<string, SeatSummary>();
    for (const d of defs) {
        if (!d.seat || !isSeatRole(d.seat.role)) continue;
        const leadId = d.seat.lead?.trim();
        seats.set(d.id, {
            role: d.seat.role,
            qualifier: optionalText(d.seat.qualifier),
            // A lead on another machine is named as it was set ("AgentY@narko"), like srv's own summaries.
            lead: leadId ? { id: leadId, name: optionalText(names.get(leadId)) ?? (leadId.includes("@") ? leadId : null) } : null,
            area: optionalText(d.seat.area),
        });
    }
    return seats;
}

/**
 * The seat an agent pane's meta carries (`agent:seat`, seeded by srv at
 * agent.open), or null when it has none or the value can't be read.
 */
export function readSeatMeta(meta: Record<string, unknown> | undefined): SeatSummary | null {
    const raw = meta?.[SEAT_META_KEY];
    if (raw == null || typeof raw !== "object") return null;
    const v = raw as Record<string, unknown>;
    if (!isSeatRole(v.role)) return null;
    let lead: SeatLead | null = null;
    if (v.lead != null && typeof v.lead === "object") {
        const l = v.lead as Record<string, unknown>;
        if (typeof l.id === "string" && l.id) lead = { id: l.id, name: optionalText(l.name) };
    }
    return { role: v.role, qualifier: optionalText(v.qualifier), lead, area: optionalText(v.area) };
}
