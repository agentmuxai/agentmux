// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Provider-scoped slash command tables, keyed by ProviderDefinition.id.
 * The registry factory (commands/registry.ts) overlays these on top of
 * the global commands when a provider is active.
 */

import type { SlashCommand } from "../types";
import { CLAUDE_COMMANDS } from "./claude";

export const SLASH_COMMANDS_BY_PROVIDER: Record<string, SlashCommand[]> = {
    claude: CLAUDE_COMMANDS,
};
