// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The agents as command palette entries: one per agent AgentMux knows, with its seat and state as
 * secondary text. Built from the same rows the Swarm and My Agents read (the runtime store, and the
 * definitions for the seat), so a rename or a new agent shows without a reload. Pure: the live lists and
 * what a selection does are in `palette-agents.ts`.
 */

import type { CommandEntry } from "@/app/store/command-registry";
import { seatBadgeText } from "@/app/view/agent/seat-format";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { AgentRuntimeState } from "@/types/rpc/AgentRuntimeState";
import type { AgentSeat } from "@/types/rpc/AgentSeat";

export const AGENT_CATEGORY = "Agent";

/** How each state reads in the palette (the same words as the status chip). */
const STATE_TEXT: Record<AgentRuntimeState, string> = {
    stopped: "stopped",
    starting: "starting",
    idle: "idle",
    working: "working",
    waiting_on_user: "waiting on you",
    limited: "limited",
    signed_out: "signed out",
    error: "error",
};

export const stateText = (state: AgentRuntimeState): string => STATE_TEXT[state] ?? state;

/** "Specialist · browser engine" for a seat, "" for an agent with none. */
export function seatText(seat: Pick<AgentSeat, "role" | "qualifier" | "area"> | undefined): string {
    if (!seat) return "";
    const badge = seatBadgeText({ role: seat.role, qualifier: seat.qualifier ?? null });
    const area = seat.area?.trim();
    return area ? `${badge} · ${area}` : badge;
}

/** What the palette shows after an agent's name: its seat and area, then its state. */
export function agentDetail(row: Pick<AgentRuntime, "state">, seat: AgentSeat | undefined): string {
    return [seatText(seat), stateText(row.state)].filter(Boolean).join(" · ");
}

/**
 * One entry per runtime row. `seats` is the seat of each agent by id; `go` is what selecting one does.
 * The words after the name (seat, area, provider, state) are searchable, so "browser" finds the agent
 * that owns it.
 */
export function agentEntries(
    rows: readonly AgentRuntime[],
    seats: ReadonlyMap<string, AgentSeat | undefined>,
    go: (agent: Pick<AgentRuntime, "agent_id" | "name">) => void | Promise<unknown>
): CommandEntry[] {
    return rows.map((row) => {
        const seat = seats.get(row.agent_id);
        return {
            id: `agent:${row.agent_id}`,
            label: row.name,
            category: AGENT_CATEGORY,
            icon: row.icon || "robot",
            iconColor: row.color,
            detail: agentDetail(row, seat),
            keywords: [seatText(seat), row.provider, stateText(row.state)].filter(Boolean).join(" "),
            execute: () => go({ agent_id: row.agent_id, name: row.name }),
        };
    });
}
