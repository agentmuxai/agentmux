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
    /**
     * Alternate text representations that resolve to this choice when
     * typed inline. The picker only displays `value`; aliases keep
     * PR #378 backwards-compat (e.g. `/model claude-sonnet` → `sonnet`).
     */
    aliases?: string[];
}

/**
 * Spec passed to `ctx.openPicker(...)`. The dispatcher builds this
 * from a command's enum/dynamic arg when the user submits a bare
 * `/cmd` and the command requires an argument. The picker UI reads
 * choices and resolves the promise with the selected value (or
 * rejects on dismiss).
 */
export interface SlashPickerSpec {
    title: string;
    choices: SlashChoice[];
}

/**
 * Argument contract for a slash command. The dispatcher uses this to
 * decide whether to open a picker, error, or dispatch immediately.
 */
export type SlashArg =
    | { kind: "none" }
    | {
          kind: "enum";
          /**
           * Either a static array of choices or a factory that receives ctx
           * and returns the choices fresh on every invocation. The factory
           * form lets commands mark the currently-active option (`current:
           * true`) by reading reactive state at picker-open time — e.g.
           * `/model` highlights the active model in the picker.
           */
          choices: SlashChoice[] | ((ctx: SlashCommandContext) => SlashChoice[]);
          required: boolean;
          defaultLabel?: string;
      }
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
    /**
     * Clear stale auth-recovery UI (the mount-time "Log in" bar, any
     * lingering `authNotice`) once a command has independently confirmed
     * the credential is good — `useAgentControllerStatus`'s
     * `notifyControllerHealthy`. /login succeeding is exactly the "agent
     * became healthy through a different path" case that function's own
     * doc comment describes: unlike `relogin()` (wired to the mount-time
     * button), /login never otherwise touches `canRetry`, so without this
     * call a pane that already showed "Log in" before the user typed
     * /login directly would have every subsequent normal message
     * fast-failed by `deliverToBackend`'s guard forever — the credential
     * is fixed, but nothing told the pane. Codex P1 on PR #2338.
     */
    notifyControllerHealthy: () => void;
    /**
     * Clear a live "auth"-classified failure row (state.failure), if any.
     * relogin()/useGlobalLogin()/loginViaTerminal() all clear this via their
     * onRecovered callback before auto-retrying, but /login is a fully
     * separate path with no equivalent — without this, a stale pre-existing
     * failure survives a successful /login, and the caller's NEXT normal
     * send re-captures that stale failure as `authFailureToPreserve`,
     * fast-failing the message and re-showing the stale banner even though
     * the credential is now fine. reagent P1 on PR #2338.
     */
    clearAuthFailure: () => void;
    /**
     * Restart an already-running persistent controller so a just-refreshed
     * credential actually takes effect — `send_message` only spawns a
     * fresh process when one isn't already running. relogin()/
     * useGlobalLogin()/loginViaTerminal() all call the equivalent
     * (`useAgentControllerStatus.forceControllerRefresh`) before declaring
     * success; /login must do the same, or a pane whose controller was
     * already alive stays on the stale credential and the next message
     * bypasses every guard in this file (nothing left to fast-fail on)
     * while still reaching that stale process. Codex P1 on PR #2338
     * (seventh re-review). Best-effort — never throws.
     */
    forceControllerRefresh: () => Promise<void>;
    /**
     * Register /login's own up-to-5-minute poll as an in-flight recovery
     * attempt, feeding the same shared counter behind
     * `useAgentControllerStatus`'s `loginWaiting()` that
     * relogin()/useGlobalLogin()/loginViaTerminal() already use. Without
     * this, `loginWaiting()` reads `false` for the whole duration of a
     * /login attempt — a second message sent while it's still polling gets
     * held with `authWasKnownBadAtQueueTime: false` (mid-turn "auth"
     * failures don't set `canRetry` either), so a /login that ultimately
     * fails flushes that held message straight to the still-known-bad
     * controller. Codex P1 on PR #2338 (ninth re-review). Must be paired
     * with exactly one `endRecoveryFlow()` call (a `finally` block).
     */
    beginRecoveryFlow: () => void;
    /** Pairs with `beginRecoveryFlow` — see its doc comment. */
    endRecoveryFlow: () => void;
    /**
     * Open the inline picker. Returns a promise that resolves with the
     * selected value, or rejects if the user dismisses (Esc / click-outside).
     * Set by useAgentCommands; the dispatcher only sees the function.
     */
    openPicker: (spec: SlashPickerSpec) => Promise<string>;
    /**
     * Show the slash command help panel. Wired by useAgentCommands to
     * a `helpVisible` signal that AgentPresentationView reads.
     */
    openHelp: () => void;
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
