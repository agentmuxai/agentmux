// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /help — open the slash command reference panel.
 *
 * Step 4 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * The handler doesn't render the panel itself — it calls ctx.openHelp(),
 * which flips a signal in useAgentCommands that AgentPresentationView
 * reads to mount <SlashHelpPanel />.
 */

import type { SlashCommand, SlashResult } from "../types";

export const helpCommand: SlashCommand = {
    name: "help",
    aliases: ["?"],
    category: "help",
    description: "Show available slash commands",
    arg: { kind: "none" },
    handler: async (ctx): Promise<SlashResult> => {
        ctx.openHelp();
        return { kind: "ok" };
    },
};
