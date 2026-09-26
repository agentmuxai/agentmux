// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer's `!cmd` prefix: a message whose first non-whitespace
 * character is `!` runs as a shell command instead of going to the agent.
 *
 * One definition for every site that asks — the send path
 * (useAgentCommands.ts), the drawer auto-open (agent-view.tsx) and the input
 * highlight (AgentFooter.tsx). They used to disagree: the first two trimmed
 * leading whitespace and the highlight didn't, so `  !ls` ran as a shell
 * command while the box showed it as an ordinary message.
 */

/** The command after `!` (trimmed; `""` for a bare `!`), or `null` if `message` isn't one. */
export function parseBangCommand(message: string): string | null {
    const trimmed = message.trimStart();
    if (!trimmed.startsWith("!")) return null;
    return trimmed.slice(1).trim();
}

export function isBangCommand(message: string): boolean {
    return parseBangCommand(message) !== null;
}
