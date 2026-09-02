// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SlashCommandRegistry — map of name → command, plus a factory that builds
 * one for a given provider.
 *
 * Step 1 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * The dispatcher, picker, autocomplete, and /help panel all read from the
 * same registry, so adding a new command is a single file registration —
 * see commands/global/*.ts and commands/providers/*.ts.
 */

import type { ProviderDefinition } from "../providers";
import type { SlashCommand, SlashCommandContext, SlashAvailability } from "./types";
import { registerGlobalCommands } from "./global";
import { SLASH_COMMANDS_BY_PROVIDER } from "./providers";

export class SlashCommandRegistry {
    private commands = new Map<string, SlashCommand>();

    /**
     * Register a command. Collisions log a warning and keep the first
     * registration — this way a provider can't accidentally shadow a
     * global command (e.g. /clear). If the second registration is
     * intentional, unregister the first explicitly.
     */
    register(cmd: SlashCommand): void {
        if (this.commands.has(cmd.name)) {
            // eslint-disable-next-line no-console
            console.warn(`[slash] duplicate registration for /${cmd.name} — keeping first`);
            return;
        }
        this.commands.set(cmd.name, cmd);
        for (const alias of cmd.aliases ?? []) {
            if (!this.commands.has(alias)) {
                this.commands.set(alias, cmd);
            }
        }
    }

    /** Case-insensitive lookup by name or alias. */
    lookup(name: string): SlashCommand | undefined {
        return this.commands.get(name.toLowerCase());
    }

    /** Iterate over unique commands (de-duped across aliases). */
    all(): SlashCommand[] {
        const seen = new Set<SlashCommand>();
        for (const cmd of this.commands.values()) seen.add(cmd);
        return Array.from(seen);
    }

    /**
     * Commands available in the given context. Filters by
     * `availability` against whether the pane has an active agent.
     */
    list(ctx: SlashCommandContext): SlashCommand[] {
        const hasAgent = Boolean(ctx.block()?.meta?.["agentId"]);
        const providerId = ctx.provider()?.id;
        return this.all().filter((cmd) => isAvailable(cmd.availability, hasAgent, providerId));
    }

    /**
     * Autocomplete — commands whose name or primary alias starts with
     * the given prefix (lowercase-insensitive). Returns unique commands
     * sorted by category then name.
     *
     * Currently called on every keystroke from AgentFooter.handleInput.
     * The matcher operates on a small fixed list (~tens of entries) so
     * per-call cost is negligible. If a future source pushes this past
     * ~50 entries (history, semantic match, agent-specific commands),
     * time-slice with scheduler.yield() per SPEC_INPUT_RESPONSIVENESS
     * §6.3 — don't ship a blocking matcher into the keystroke path.
     */
    completions(prefix: string, ctx: SlashCommandContext): SlashCommand[] {
        const p = prefix.toLowerCase();
        return this.list(ctx)
            .filter((cmd) => cmd.name.startsWith(p) || (cmd.aliases ?? []).some((a) => a.startsWith(p)))
            .sort((a, b) => {
                if (a.category !== b.category) return a.category.localeCompare(b.category);
                return a.name.localeCompare(b.name);
            });
    }
}

/**
 * Build a registry for a given provider. Called from useAgentCommands
 * and memoized — re-runs only when the provider changes.
 *
 * Order:
 *   1. Global commands (always available)
 *   2. Provider-scoped commands (if a provider is active)
 */
export function buildRegistry(provider: ProviderDefinition | undefined): SlashCommandRegistry {
    const registry = new SlashCommandRegistry();

    registerGlobalCommands(registry);

    if (provider) {
        const providerCommands = SLASH_COMMANDS_BY_PROVIDER[provider.id] ?? [];
        for (const cmd of providerCommands) {
            registry.register(cmd);
        }
    }

    return registry;
}

function isAvailable(
    availability: SlashAvailability | undefined,
    hasAgent: boolean,
    providerId: string | undefined,
): boolean {
    if (availability === undefined || availability === "global") return true;
    if (availability === "any-agent") return hasAgent;
    if (availability === "picker-only") return !hasAgent;
    if (typeof availability === "object" && "provider" in availability) {
        return availability.provider === providerId;
    }
    return false;
}
