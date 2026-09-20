// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Claude-specific slash commands. Step 5 of
 * docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md eventually adds
 * /cost, /status, /doctor, /memory, /hooks, /mcp, /config, /compact, /bug,
 * /release-notes here too.
 */

import type { SlashCommand } from "../types";
import { btwCommand } from "./btw";

export const CLAUDE_COMMANDS: SlashCommand[] = [btwCommand];
