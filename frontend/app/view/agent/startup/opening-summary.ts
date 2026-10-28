// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The opening update: when the person opens an agent's pane, the agent starts with 1 to 3
 * lines on what it did since their last message to it, what is in flight and what it waits on.
 *
 * Only an explicit open asks for it (`openAgentInActiveTab`: My Agents, the Swarm's Open, the
 * picker), never what merely mounts a pane: a layout restore after a restart, a switch to a tab
 * whose panes were hidden, or a background resume. Otherwise every agent in the active tab would
 * spend a turn nobody reads each time AgentMux starts. The open notes the agent here before it
 * asks srv, because the pane can mount before the answer comes back; the pane takes the note once
 * when it connects (`sendStartupSequence`).
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

/**
 * Sent as the opening turn, for native and container agents alike. Its heading is the one the
 * startup payload has, so the transcript shows it collapsed as setup rather than as the
 * person's own words (stream-parser.ts, "# Session Context").
 */
export const OPENING_SUMMARY_PROMPT = `# Session Context

## Opening update

The person has just opened your pane. Open with an update of at most three short lines, and nothing else: what you did since their last message to you (tasks, pull requests, merges), what is in flight, and what you are waiting on. Use your own history: this session, your memory, and the agent history search if the session is new to you. If you have nothing yet, say "New agent, nothing yet." Run the usual setup check only if something looks wrong.
`;
