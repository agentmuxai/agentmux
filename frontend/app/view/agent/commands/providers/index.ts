// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Provider-scoped slash command tables, keyed by ProviderDefinition.id.
 * The registry factory (commands/registry.ts) overlays these on top of
 * the global commands when a provider is active.
 */

import type { SlashCommand } from "../types";
import { btwCommand } from "./btw";
import { CLAUDE_COMMANDS } from "./claude";
import { OPENCLAW_COMMANDS } from "./openclaw";

export const SLASH_COMMANDS_BY_PROVIDER: Record<string, SlashCommand[]> = {
    claude: CLAUDE_COMMANDS,
    // `/btw` is AgentMux's own side question, not a pass-through to a CLI
    // command, so it runs on these CLIs too (side_question.rs "Providers").
    codex: [btwCommand],
    gemini: [btwCommand],
    openclaw: OPENCLAW_COMMANDS,
};
