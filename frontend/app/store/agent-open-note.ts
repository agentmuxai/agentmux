// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The note an explicit open leaves for the agent's pane: when the person opens an agent
 * (`openAgentInActiveTab`: My Agents, the Swarm's Open, the picker) the pane that mounts for it
 * opens with an update (`view/agent/startup/opening-summary.ts`). Only that open leaves a note,
 * never what merely mounts a pane (a layout restore, a tab switch, a background resume).
 *
 * It is noted before the open asks srv, because the pane can mount before the answer comes back,
 * and keyed by agent because the block does not exist yet. The pane takes it once when it connects.
 */

/** How long an open waits for its pane to mount: an open that never mounts one (it focused a pane that was already there) expires. */
export const OPEN_NOTE_TTL_MS = 60_000;

const opened = new Map<string, number>();

/** The person opened this agent now. */
export function noteOpenedByPerson(agentId: string, now = Date.now()): void {
    opened.set(agentId, now);
}

/** Whether the person opened this agent and its pane has not asked yet; asking uses the note up. */
export function takeOpenedByPerson(agentId: string | undefined, now = Date.now()): boolean {
    if (!agentId) return false;
    const at = opened.get(agentId);
    opened.delete(agentId);
    return at !== undefined && now - at <= OPEN_NOTE_TTL_MS;
}
