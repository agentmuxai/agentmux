// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * dispatchSlashCommand — the single entry point for handling `/cmd arg`
 * composer input.
 *
 * Step 1 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * The caller (useAgentCommands.sendMessage) passes the raw composer input
 * and a context bundle. The dispatcher:
 *
 *   1. Parses the input into a command name + argument string.
 *   2. Looks up the command in the registry.
 *      - Unknown → "passthrough" (caller falls through to AgentInputCommand).
 *   3. Validates the argument against the command's arg contract.
 *      - Missing required enum arg → error (picker UI lands in step 2).
 *      - Missing required freeform → error with placeholder hint.
 *      - Enum value mismatch → error listing allowed values.
 *   4. Invokes the handler with the validated arg.
 *   5. Formats the result:
 *      - "ok" → log the success message (if any) at info level
 *      - "error" → log at warn level
 *      - "passthrough" → caller falls through
 *
 * Handlers are pure async functions — they return a SlashResult and the
 * dispatcher centralizes all user-visible logging. This is intentional:
 * handlers shouldn't decide where to render, and they shouldn't know
 * about the launch-log sink.
 */

import { parseSlashCommand } from "./parse";
import type { SlashChoice, SlashCommand, SlashCommandContext, SlashResult } from "./types";
import type { SlashCommandRegistry } from "./registry";

export type DispatchOutcome =
    | { kind: "handled" } // command ran (ok or error) — do NOT fall through
    | { kind: "passthrough" }; // no matching command — caller sends as user message

/**
 * Dispatch a slash command. Always returns either "handled" (caller
 * returns immediately from sendMessage) or "passthrough" (caller
 * proceeds to AgentInputCommand).
 */
export async function dispatchSlashCommand(
    input: string,
    registry: SlashCommandRegistry,
    ctx: SlashCommandContext,
): Promise<DispatchOutcome> {
    const [name, arg] = parseSlashCommand(input);
    if (!name) return { kind: "passthrough" };

    const cmd = registry.lookup(name);
    if (!cmd) return { kind: "passthrough" };

    const result = await runValidatedHandler(cmd, arg, ctx);
    formatResult(cmd, result, ctx);
    return { kind: "handled" };
}

/**
 * Validate the arg against the command's arg contract, then invoke the
 * handler. Returns the handler's SlashResult (or a validation-error
 * result if the arg didn't pass).
 *
 * For required enum args with no value, opens an inline picker via
 * ctx.openPicker (step 2 of the spec). The picker resolves with the
 * selected value or rejects on dismissal — dismissal is surfaced as
 * a silent ok so we don't spam the log.
 */
async function runValidatedHandler(
    cmd: SlashCommand,
    arg: string,
    ctx: SlashCommandContext,
): Promise<SlashResult> {
    if (cmd.arg.kind === "none") {
        return cmd.handler(ctx, "");
    }

    if (cmd.arg.kind === "enum") {
        const choices = typeof cmd.arg.choices === "function" ? cmd.arg.choices(ctx) : cmd.arg.choices;
        if (arg === "") {
            if (cmd.arg.required) {
                try {
                    const picked = await ctx.openPicker({
                        title: `Select ${cmd.name}`,
                        choices,
                    });
                    return cmd.handler(ctx, picked);
                } catch {
                    return { kind: "ok" };
                }
            }
            return cmd.handler(ctx, "");
        }
        const match = matchEnumChoice(choices, arg);
        if (!match) {
            return {
                kind: "error",
                message: `/${cmd.name}: unknown value '${arg}'. Try: ${choices
                    .map((c) => c.value)
                    .join(" | ")}`,
            };
        }
        return cmd.handler(ctx, match.value);
    }

    if (cmd.arg.kind === "freeform") {
        if (arg === "" && cmd.arg.required) {
            return {
                kind: "error",
                message: `/${cmd.name} requires an argument: ${cmd.arg.placeholder}`,
            };
        }
        return cmd.handler(ctx, arg);
    }

    // kind === "dynamic" — resolve completions and open picker if empty.
    if (arg === "") {
        try {
            const choices = await cmd.arg.completions(ctx);
            const picked = await ctx.openPicker({
                title: `Select ${cmd.name}`,
                choices,
            });
            return cmd.handler(ctx, picked);
        } catch {
            return { kind: "ok" };
        }
    }
    return cmd.handler(ctx, arg);
}

/**
 * Match an enum arg against its choices. Tries exact value match
 * first, then aliases. Case-insensitive on both sides.
 */
function matchEnumChoice(choices: SlashChoice[], arg: string): SlashChoice | undefined {
    const a = arg.toLowerCase();
    return choices.find(
        (c) =>
            c.value.toLowerCase() === a ||
            (c.aliases ?? []).some((alias) => alias.toLowerCase() === a),
    );
}

/**
 * Centralized result logging. Handlers never call ctx.log directly;
 * they return a SlashResult and the dispatcher decides how to surface it.
 */
function formatResult(
    cmd: SlashCommand,
    result: SlashResult,
    ctx: SlashCommandContext,
): void {
    if (result.kind === "ok") {
        if (result.message) {
            ctx.log("system", result.message);
        }
        return;
    }
    if (result.kind === "error") {
        ctx.log("system", result.message, "warn");
        return;
    }
    // passthrough from a handler is a noop here — the dispatcher returns
    // handled:true, so the caller is about to suppress AgentInputCommand.
    // Handlers returning passthrough is an escape hatch; at present no
    // registered command does.
    ctx.log("system", `/${cmd.name}: passthrough requested (ignored)`, "warn");
}
