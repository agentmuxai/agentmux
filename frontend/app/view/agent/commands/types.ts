// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Slash-command architecture — type definitions.
 *
 * Step 1 of specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * Commands are data. This file defines the shape. The dispatcher, picker,
 * autocomplete, and help panel all consume the same registry, so adding a
 * new command is one file create — no touches to any presentation or
 * dispatch code.
 *
 * See the spec §4 for the design rationale.
 */

import type { ProviderDefinition } from "../providers";
import type { SignalPair } from "../state";
import type { DocumentNode } from "../types";
import type { LogFn } from "../hooks/useAgentControllerStatus";

/** Grouping used by /help and (eventually) the settings browser. */
export type SlashCommandCategory = "runtime" | "session" | "auth" | "query" | "system" | "help";

/** A picker choice surface. Used by enum + dynamic arg kinds. */
export interface SlashChoice {
    value: string;
    label: string;
    description?: string;
    /** Render this option as the currently-active selection. */
    current?: boolean;
}

/**
 * Argument contract for a slash command. The dispatcher uses this to
 * decide whether to open a picker, error, or dispatch immediately.
 */
export type SlashArg =
    | { kind: "none" }
    | { kind: "enum"; choices: SlashChoice[]; required: boolean; defaultLabel?: string }
    | { kind: "freeform"; placeholder: string; required: boolean }
    | {
          kind: "dynamic";
          placeholder: string;
          completions: (ctx: SlashCommandContext) => Promise<SlashChoice[]>;
      };

/**
 * Where a command is allowed to run. Undefined = always available.
 *
 *   "global"        — always available (e.g. /help, /clear, /login)
 *   "any-agent"     — only when block.meta.agentId is set (pane has launched)
 *   "picker-only"   — only on the pre-launch agent picker screen
 *   { provider: X } — only when the active provider.id matches
 */
export type SlashAvailability =
    | "global"
    | "any-agent"
    | "picker-only"
    | { provider: string };

/** Outcome of dispatching a command. */
export type SlashResult =
    | { kind: "ok"; message?: string }
    | { kind: "error"; message: string }
    /**
     * The dispatcher should fall through to AgentInputCommand — i.e. treat
     * the input as a regular user message. Used when the command matches
     * but the handler decides the user didn't actually mean to invoke it
     * (rare; mostly a future escape hatch).
     */
    | { kind: "passthrough" };

/**
 * Context passed to every command handler. A grab bag of the things a
 * handler might need — the dispatcher builds it once per invocation.
 *
 * Intentionally small: if a command needs something not here, add it in
 * the spec and this type together, not hidden behind a closure in the
 * handler file.
 */
export interface SlashCommandContext {
    /** Block id this command runs against. */
    blockId: string;
    /** Current provider definition, if the pane is in presentation mode. */
    provider: () => ProviderDefinition | undefined;
    /** Reactive accessor for the current block meta. */
    block: () => { meta?: Record<string, any> } | undefined;
    /** Document atom pair for commands that mutate the conversation (/clear). */
    documentAtom: SignalPair<DocumentNode[]>;
    /** Launch-log sink for system messages. */
    log: LogFn;
    /** Set the OAuth URL for /login. */
    setAuthUrl: (url: string | null) => void;
}

/**
 * Shape of a single slash command. Handlers are pure async functions
 * that return a SlashResult — no direct logging, no direct UI calls.
 * The dispatcher centralizes result formatting.
 */
export interface SlashCommand {
    /** Primary name. Always lowercase, no leading slash. */
    name: string;
    /** Alternate names. Lookup is case-insensitive. */
    aliases?: string[];
    /** Grouping for /help and autocomplete. */
    category: SlashCommandCategory;
    /** One-line description. Shown in /help and autocomplete. */
    description: string;
    /** Optional longer help text (also Markdown-safe). */
    longDescription?: string;
    /** Argument contract. */
    arg: SlashArg;
    /** Where the command is allowed to run. */
    availability?: SlashAvailability;
    /** The actual handler. Called AFTER arg validation / picker resolution. */
    handler: (ctx: SlashCommandContext, arg: string) => Promise<SlashResult>;
}
