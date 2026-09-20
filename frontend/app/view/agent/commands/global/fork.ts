// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /fork — fork this pane's live agent into a new sibling pane-stack tab.
 * Slash-command entry point for the same action the pane's right-click
 * "Quick Fork" context-menu item already triggers (`quick-fork.ts`'s
 * `quickForkAgent`, wired via `ctx.quickFork`) — full independent identity,
 * conversation history carried forward. See `quickForkAgent`'s own doc
 * comment for the fork's exact semantics (identity binding, non-Claude
 * history fallback, failure cleanup).
 */

import type { SlashCommand, SlashResult } from "../types";

export const forkCommand: SlashCommand = {
    name: "fork",
    category: "session",
    description: "Fork this conversation into a new sibling tab",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const launched = await ctx.quickFork();
        if (!launched) {
            // quickForkAgent already surfaces a toast + logs its own
            // failure reason — this is just the dispatcher-visible echo.
            return { kind: "error", message: "/fork: could not fork this conversation" };
        }
        return { kind: "ok" };
    },
};
