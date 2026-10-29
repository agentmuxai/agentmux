// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The opening update: when the person opens an agent's pane, the agent starts with 1 to 3
 * lines on what it did since their last message to it, what is in flight and what it waits on.
 * Only an explicit open asks for it (`store/agent-open-note.ts`), never what merely mounts a
 * pane, so a restart does not make every agent in the active tab spend a turn nobody reads.
 */

/**
 * Sent as the opening turn, for native and container agents alike. Its heading is the one the
 * startup payload has, so the transcript shows it collapsed as setup rather than as the
 * person's own words (stream-parser.ts, "# Session Context").
 */
export const OPENING_SUMMARY_PROMPT = `# Session Context

## Opening update

The person has just opened your pane. Open with an update of at most three short lines, and nothing else: what you did since their last message to you (tasks, pull requests, merges), what is in flight, and what you are waiting on. Use your own history: this session, your memory, and the agent history search if the session is new to you. If you have nothing yet, say "New agent, nothing yet." Run the usual setup check only if something looks wrong.
`;
