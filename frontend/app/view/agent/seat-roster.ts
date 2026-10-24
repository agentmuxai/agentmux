// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The roster import behind the Seat tab's "Import roster…"
 * (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3): a small JSON of
 * agent (name or uid) → role, qualifier, lead, area. Each agent, and each
 * lead, is resolved by uid or by exact name; anything that doesn't resolve
 * to exactly one agent is listed with the reason, never guessed.
 *
 * Accepted shapes:
 *   { "AgentX": { "role": "leader", "qualifier": "product", "lead": null, "area": "agent pane" }, … }
 *   [ { "agent": "AgentX", "role": "leader", … }, … ]
 * A lead of null, "", "operator" or "the operator" means the operator.
 */

import type { SeatInput } from "@/types/rpc/SeatInput";
import { isSeatRole, MAX_AREA_CHARS, MAX_QUALIFIER_CHARS } from "./seat-format";

/** An agent a roster line can name: its uid and its name. */
export interface RosterAgent {
    id: string;
    name: string;
}

/** One roster line as written. */
export interface RosterEntry {
    agent: string;
    role: unknown;
    qualifier?: unknown;
    lead?: unknown;
    area?: unknown;
}

/** A line that resolved: the agent, the seat to set, and its lead's name for the preview. */
export interface ResolvedSeat {
    ref: string;
    agent: RosterAgent;
    seat: SeatInput;
    /** The lead's name, or "the operator". */
    leadName: string;
}

/** A line that didn't, with why. */
export interface UnresolvedLine {
    ref: string;
    reason: string;
}

export interface RosterPlan {
    seats: ResolvedSeat[];
    unresolved: UnresolvedLine[];
}

const OPERATOR_WORDS = new Set(["", "operator", "the operator"]);

/** Parse the JSON into lines; throws an Error with a readable message when it isn't a roster. */
export function parseRoster(text: string): RosterEntry[] {
    let data: unknown;
    try {
        data = JSON.parse(text);
    } catch (e) {
        throw new Error(`Not valid JSON: ${e instanceof Error ? e.message : String(e)}`);
    }
    const asEntry = (agent: string, v: unknown): RosterEntry => {
        if (v == null || typeof v !== "object" || Array.isArray(v)) {
            throw new Error(`The seat for "${agent}" isn't an object`);
        }
        const o = v as Record<string, unknown>;
        return { agent, role: o.role, qualifier: o.qualifier, lead: o.lead, area: o.area };
    };
    if (Array.isArray(data)) {
        return data.map((v, i) => {
            const o = (v ?? {}) as Record<string, unknown>;
            const agent = o.agent ?? o.name ?? o.id;
            if (typeof agent !== "string" || !agent.trim()) throw new Error(`Line ${i + 1} doesn't name an agent`);
            return asEntry(agent.trim(), v);
        });
    }
    if (data != null && typeof data === "object") {
        return Object.entries(data as Record<string, unknown>).map(([agent, v]) => asEntry(agent.trim(), v));
    }
    throw new Error("A roster is an object of agent → seat, or a list of seats");
}

type Lookup = { agent: RosterAgent } | { reason: string };

/** An agent by uid, else by exact name; more than one with the name is not a match. */
function lookup(ref: string, agents: RosterAgent[]): Lookup {
    const byId = agents.find((a) => a.id === ref);
    if (byId) return { agent: byId };
    const byName = agents.filter((a) => a.name === ref);
    if (byName.length === 1) return { agent: byName[0] };
    if (byName.length > 1) return { reason: `${byName.length} agents are named "${ref}"` };
    return { reason: `no agent is named "${ref}" or has that uid` };
}

const text = (v: unknown): string | undefined => (typeof v === "string" && v.trim() ? v.trim() : undefined);

/** Resolve each line against this machine's agents (templates excluded by the caller). */
export function resolveRoster(entries: RosterEntry[], agents: RosterAgent[]): RosterPlan {
    const seats: ResolvedSeat[] = [];
    const unresolved: UnresolvedLine[] = [];
    const seen = new Set<string>();
    for (const entry of entries) {
        const ref = entry.agent;
        const found = lookup(ref, agents);
        if ("reason" in found) {
            unresolved.push({ ref, reason: found.reason });
            continue;
        }
        if (seen.has(found.agent.id)) {
            unresolved.push({ ref, reason: `${found.agent.name} is listed more than once` });
            continue;
        }
        if (!isSeatRole(entry.role)) {
            unresolved.push({ ref, reason: `unknown role "${String(entry.role ?? "")}"` });
            continue;
        }
        const qualifier = text(entry.qualifier);
        const area = text(entry.area);
        if (qualifier && qualifier.length > MAX_QUALIFIER_CHARS) {
            unresolved.push({ ref, reason: `the qualifier is over ${MAX_QUALIFIER_CHARS} characters` });
            continue;
        }
        if (area && area.length > MAX_AREA_CHARS) {
            unresolved.push({ ref, reason: `the area is over ${MAX_AREA_CHARS} characters` });
            continue;
        }
        let lead: RosterAgent | null = null;
        const leadRef = entry.lead == null ? "" : String(entry.lead).trim();
        if (!OPERATOR_WORDS.has(leadRef.toLowerCase())) {
            const leadFound = lookup(leadRef, agents);
            if ("reason" in leadFound) {
                unresolved.push({ ref, reason: `its lead: ${leadFound.reason}` });
                continue;
            }
            if (leadFound.agent.id === found.agent.id) {
                unresolved.push({ ref, reason: "an agent can't be its own lead" });
                continue;
            }
            lead = leadFound.agent;
        }
        seen.add(found.agent.id);
        seats.push({
            ref,
            agent: found.agent,
            seat: { role: entry.role, qualifier, lead: lead?.id, area },
            leadName: lead ? lead.name : "the operator",
        });
    }
    return { seats, unresolved };
}

/**
 * The seats in the order to set them: a lead's own seat before its reports',
 * so a roster that moves leads around doesn't trip srv's cycle check on
 * the way. Lines the roster doesn't chain keep their order.
 */
export function orderForApply(seats: ResolvedSeat[]): ResolvedSeat[] {
    const byAgent = new Map(seats.map((s) => [s.agent.id, s]));
    const depth = new Map<string, number>();
    const depthOf = (s: ResolvedSeat, path: Set<string>): number => {
        const known = depth.get(s.agent.id);
        if (known != null) return known;
        const lead = s.seat.lead ? byAgent.get(s.seat.lead) : undefined;
        // A loop inside the roster: srv refuses one line of it with its own message.
        const d = lead && !path.has(lead.agent.id) ? depthOf(lead, new Set(path).add(s.agent.id)) + 1 : 0;
        depth.set(s.agent.id, d);
        return d;
    };
    return seats
        .map((s, i) => ({ s, i, d: depthOf(s, new Set([s.agent.id])) }))
        .sort((a, b) => a.d - b.d || a.i - b.i)
        .map((x) => x.s);
}
