// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Claude-specific slash commands. Empty in step 1 — step 5 of
 * docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md populates this
 * with /cost, /status, /doctor, /memory, /hooks, /mcp, /config,
 * /compact, /bug, /release-notes.
 */

import type { SlashCommand } from "../types";

export const CLAUDE_COMMANDS: SlashCommand[] = [];
