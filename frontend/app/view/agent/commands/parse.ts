// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Parse `/command arg1 arg2 ...` composer input into `[name, argString]`.
 *
 * Part of the slash command architecture —
 * specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * The dispatcher hands the full argument string to the command handler;
 * commands that want to tokenize further do so themselves. This keeps
 * the parser trivial and avoids the "quoted arg" rabbit hole that shell
 * parsers grow over time — slash commands don't need shell-grade
 * quoting because their args are either enums or short freeform text.
 */

/**
 * Parse `/command arg` → `["command", "arg"]`. Leading slash stripped,
 * command name lowercased, arg trimmed. For bare `/command`, returns
 * `["command", ""]`. For input that doesn't start with `/`, returns
 * `["", ""]`.
 */
export function parseSlashCommand(input: string): [string, string] {
    const trimmed = input.trim();
    if (!trimmed.startsWith("/")) return ["", ""];
    const rest = trimmed.slice(1);
    const spaceIdx = rest.indexOf(" ");
    if (spaceIdx < 0) return [rest.toLowerCase(), ""];
    return [rest.slice(0, spaceIdx).toLowerCase(), rest.slice(spaceIdx + 1).trim()];
}
