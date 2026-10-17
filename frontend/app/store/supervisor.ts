// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm Supervisor, as the frontend reads it: a pane's mode
 * (`supervisor:mode`) and the run state srv writes after every decision
 * (`supervisor:state`). srv owns both; the frontend sets the mode only through
 * `supervisor.set-mode`. docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md.
 */

import type { SupervisorMode } from "@/types/rpc/SupervisorMode";
import type { SupervisorState } from "@/types/rpc/SupervisorState";

export const META_SUPERVISOR_MODE = "supervisor:mode";
export const META_SUPERVISOR_STATE = "supervisor:state";

/** The header srv puts on a continuation (`supervisor_turn_message`). */
export const SUPERVISOR_HEADER_PREFIX = "[SUPERVISOR:";

export const SUPERVISOR_MODES: readonly SupervisorMode[] = ["off", "steady", "drive"];

export const MODE_LABELS: Record<SupervisorMode, string> = { off: "Off", steady: "Steady", drive: "Drive" };

export const MODE_DESCRIPTIONS: Record<SupervisorMode, string> = {
    off: "Leave the agent to stop when it stops.",
    steady: "Keep it going when it stops only to check in. Pause for real questions.",
    drive: "Keep it going through open questions on its own best judgement. Pause for what only you can provide.",
};

type Meta = Record<string, unknown> | undefined;

/** The pane's mode; anything unset or unknown is Off, as srv reads it. */
export function readSupervisorMode(meta: Meta): SupervisorMode {
    const v = meta?.[META_SUPERVISOR_MODE];
    return v === "steady" || v === "drive" ? v : "off";
}

/** The run state srv wrote, or null. */
export function readSupervisorState(meta: Meta): SupervisorState | null {
    const v = meta?.[META_SUPERVISOR_STATE];
    if (!v || typeof v !== "object" || !Array.isArray((v as SupervisorState).history)) return null;
    return v as SupervisorState;
}

/** Whether the Supervisor works on this pane: Claude agents only, for now. */
export function supervisorSupported(meta: Meta): boolean {
    return meta?.["agentProvider"] === "claude";
}

/**
 * Why the Supervisor stopped this agent for the user, or null. Only while the
 * agent is idle: once a turn is in flight (the user answered, or anything else
 * started it), the pause is over even before srv's next decision.
 */
export function supervisorPauseReason(meta: Meta, live: boolean): string | null {
    if (live || readSupervisorMode(meta) === "off") return null;
    const state = readSupervisorState(meta);
    if (state?.status !== "paused") return null;
    return state.reason?.trim() || "The Supervisor stopped for you.";
}

/** The chip's short state: "judging", "3/15", "paused", … or null for none. */
export function supervisorChipDetail(state: SupervisorState | null, mode: SupervisorMode, live: boolean): string | null {
    if (mode === "off" || !state || state.mode !== mode) return null;
    switch (state.status) {
        case "judging":
            return "judging";
        case "waiting":
            return "waiting";
        case "paused":
            return live ? null : "paused";
        case "done":
            return live ? null : "done";
        case "continued":
            return state.continuations > 0 ? `${state.continuations}/${state.limit}` : null;
        default:
            return null;
    }
}

/** A continuation's header split off a user message, or null for any other
 *  message. Only the exact shape srv writes counts. */
export function parseSupervisorHeader(
    message: string
): { body: string; mode: "steady" | "drive"; n: number; of: number } | null {
    const match = /^\[SUPERVISOR:MODE=(steady|drive) N=(\d+) OF=(\d+)\]\r?\n([\s\S]*)$/.exec(message.trim());
    if (!match) return null;
    return { mode: match[1] as "steady" | "drive", n: Number(match[2]), of: Number(match[3]), body: match[4] };
}
