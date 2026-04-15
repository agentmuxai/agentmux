// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Global slash commands — always available regardless of provider.
 * The registry factory (commands/registry.ts) calls this first, then
 * overlays any provider-scoped commands.
 */

import type { SlashCommandRegistry } from "../registry";
import { clearCommand } from "./clear";
import { helpCommand } from "./help";
import { loginCommand } from "./login";
import { RUNTIME_COMMANDS } from "./runtime";
import { toolsCommand } from "./tools";

export function registerGlobalCommands(registry: SlashCommandRegistry): void {
    for (const cmd of RUNTIME_COMMANDS) {
        registry.register(cmd);
    }
    registry.register(loginCommand);
    registry.register(clearCommand);
    registry.register(helpCommand);
    registry.register(toolsCommand);
}
