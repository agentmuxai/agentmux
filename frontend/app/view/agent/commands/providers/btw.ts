// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /btw <question> — a faithful recreation of Claude Code CLI's own native
 * `/btw`: ask a tool-less, context-aware side question in an ephemeral
 * floating overlay (`components/BtwOverlay.tsx`) without touching this
 * pane's own live turn or transcript. AgentMux's own, so it isn't limited
 * to Claude: registered for the Claude, Codex and Gemini providers
 * (`providers/index.ts`) — the backend runs each tool-less on the pane's
 * own CLI.
 *
 * The handler itself is thin — `ctx.askSideQuestion` both fires the
 * backend request AND opens the overlay (see that field's doc comment in
 * `commands/types.ts`); this handler only validates the argument and
 * translates a rejected request into a `SlashResult` error.
 */

import type { SlashCommand, SlashResult } from "../types";

export const btwCommand: SlashCommand = {
    name: "btw",
    category: "query",
    description: "Ask a side question without interrupting the current turn",
    arg: { kind: "freeform", placeholder: "<question>", required: true },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const question = arg.trim();
        if (!question) {
            return { kind: "error", message: "/btw: a question is required" };
        }
        try {
            await ctx.askSideQuestion(question);
        } catch (e) {
            // ctx.askSideQuestion already surfaced this inline in the
            // overlay it opened (see its doc comment) — this is just the
            // dispatcher-visible echo, same convention as /fork.
            const message = e instanceof Error ? e.message : String(e);
            return { kind: "error", message: `/btw: ${message}` };
        }
        return { kind: "ok" };
    },
};
