// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The agent entry's id in the status bar registry. */
export const AGENT_COUNTS_ID = "agentmux:agent-counts";

/**
 * The fit tier at which a status bar slot goes in a narrow bar. The agent entry is where a request that waits on the
 * person is kept once its toast has gone, so while anything waits it holds on to the last tier; the others keep
 * their own.
 */
export function slotDropAt(id: string, base: 1 | 2 | 3 | 4 | undefined, waiting: boolean): 1 | 2 | 3 | 4 | undefined {
    return id === AGENT_COUNTS_ID && waiting ? 4 : base;
}
