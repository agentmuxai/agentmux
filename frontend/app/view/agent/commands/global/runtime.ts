// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runtime-config slash commands: /model /effort /permission-mode /bypass
 * /plan /runtime.
 *
 * Step 2 of specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md —
 * /model, /effort, and /permission-mode are now `enum` arg kind so a
 * bare `/model` opens the inline picker. Aliases (e.g.
 * `claude-sonnet` → `sonnet`) live on each SlashChoice for backwards
 * compatibility with PR #378's parsing.
 *
 * The `choices` factory reads the current runtime config so the picker
 * marks the active option — the user opens `/model`, sees `Opus
 * (current)`, and Enter is a no-op confirmation.
 */

import { getRuntimeConfig } from "../../buildRuntimeArgs";
import { applyRuntimeChange } from "../../runtime-apply";
import type { AgentRuntimeConfig, EffortLevel, PermissionMode } from "../../types";
import type { SlashChoice, SlashCommand, SlashCommandContext, SlashResult } from "../types";

type RuntimeUpdateResult =
    | { ok: true; updated: AgentRuntimeConfig }
    | { ok: false; error: string };

/**
 * Mutate block.meta["agent:runtime"] with a partial runtime config.
 * Returns the merged config on success or the underlying error
 * message on RPC failure. Callers fold the error into a SlashResult
 * so the user sees the real reason instead of a generic "failed".
 */
async function updateRuntime(
    ctx: SlashCommandContext,
    patch: Partial<AgentRuntimeConfig>,
): Promise<RuntimeUpdateResult> {
    const current = getRuntimeConfig(ctx.block()?.meta);
    const updated: AgentRuntimeConfig = { ...current, ...patch };
    try {
        // Persist + (for persistent Claude) rebuild cmd:args & force-restart so
        // the change applies to the running agent. Shared with the GUI control
        // bar via applyRuntimeChange — the persistent rebuild/restart used to
        // live only here (#1503), so the dropdown silently no-op'd.
        await applyRuntimeChange(ctx.blockId, ctx.provider(), updated);
        return { ok: true, updated };
    } catch (err: any) {
        return { ok: false, error: err?.message ?? String(err) };
    }
}

function runtimeError(error: string): SlashResult {
    return { kind: "error", message: `failed to update runtime config: ${error}` };
}

// ── /model ────────────────────────────────────────────────────────────

function modelChoices(ctx: SlashCommandContext): SlashChoice[] {
    const current = getRuntimeConfig(ctx.block()?.meta).model;
    // Per-provider model list — Claude shows opus/sonnet/haiku, codex shows the
    // gpt-5.x line, etc. Providers without a `models` list (kimi/openclaw/pi/
    // copilot/gemini/qwen/muxcode pick their model in their own config) show no
    // choices here.
    const models = ctx.provider()?.models ?? [];
    return models.map((m) => ({
        value: m.value,
        label: m.label,
        description: m.description ?? m.value,
        current: current === m.value,
        aliases: m.aliases,
    }));
}

const modelCommand: SlashCommand = {
    name: "model",
    category: "runtime",
    description: "Change the active model (applies to next turn)",
    arg: { kind: "enum", required: true, choices: modelChoices },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const result = await updateRuntime(ctx, { model: arg });
        if (result.ok === false) return runtimeError(result.error);
        return { kind: "ok", message: `model set to ${result.updated.model} (applies to next turn)` };
    },
};

// ── /effort ───────────────────────────────────────────────────────────

function effortChoices(ctx: SlashCommandContext): SlashChoice[] {
    const current = getRuntimeConfig(ctx.block()?.meta).effort;
    const make = (
        value: string,
        label: string,
        description: string,
        effort: EffortLevel,
        aliases?: string[],
    ): SlashChoice => ({
        value,
        label,
        description,
        current: current === effort,
        aliases,
    });
    return [
        make("low", "low", "Minimal reasoning effort", "low"),
        make("medium", "medium", "Balanced reasoning effort", "medium", ["med"]),
        make("high", "high", "High reasoning effort", "high"),
        make("xhigh", "xhigh", "Best for coding/agentic (Claude Code default)", "xhigh", ["extra-high", "x-high"]),
        make("max", "max", "Maximum reasoning effort", "max"),
    ];
}

const effortCommand: SlashCommand = {
    name: "effort",
    category: "runtime",
    description: "Change reasoning effort level (applies to next turn)",
    arg: { kind: "enum", required: true, choices: effortChoices },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const result = await updateRuntime(ctx, { effort: arg as EffortLevel });
        if (result.ok === false) return runtimeError(result.error);
        return { kind: "ok", message: `effort set to ${result.updated.effort} (applies to next turn)` };
    },
};

// ── /permission-mode ──────────────────────────────────────────────────

function permissionChoices(ctx: SlashCommandContext): SlashChoice[] {
    const current = getRuntimeConfig(ctx.block()?.meta).permissionMode;
    const make = (
        value: string,
        label: string,
        description: string,
        mode: PermissionMode,
        aliases?: string[],
    ): SlashChoice => ({
        value,
        label,
        description,
        current: current === mode,
        aliases,
    });
    return [
        make("default", "Default", "Standard permission prompts", "default"),
        make("auto", "Auto", "Auto-approve safe operations", "auto"),
        make("acceptEdits", "Accept Edits", "Auto-approve file edits", "acceptEdits", [
            "accept",
            "acceptedits",
            "accept-edits",
        ]),
        make("plan", "Plan", "No tool execution — read-only planning", "plan"),
        make("bypass", "Bypass", "Skip all permission prompts (dangerous)", "bypass", [
            "dangerous",
            "skip",
            "dangerously-skip-permissions",
        ]),
    ];
}

const permissionModeCommand: SlashCommand = {
    name: "permission-mode",
    aliases: ["permission", "perm"],
    category: "runtime",
    description: "Change permission mode (applies to next turn)",
    arg: { kind: "enum", required: true, choices: permissionChoices },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const result = await updateRuntime(ctx, { permissionMode: arg as PermissionMode });
        if (result.ok === false) return runtimeError(result.error);
        return {
            kind: "ok",
            message: `permission mode set to ${result.updated.permissionMode} (applies to next turn)`,
        };
    },
};

// ── /bypass ───────────────────────────────────────────────────────────
// Shortcut: bare `/bypass` enables; `/bypass off` (or `default`) reverts.
// Not an enum because the no-arg case has the *opposite* meaning of
// /model — bare `/bypass` should DO something, not open a picker.

const bypassCommand: SlashCommand = {
    name: "bypass",
    category: "runtime",
    description: "Enable permission bypass for the next turn (dangerous)",
    arg: { kind: "freeform", placeholder: "(empty) | off | default", required: false },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const key = arg.toLowerCase();
        let patch: Partial<AgentRuntimeConfig>;
        if (key === "") {
            patch = { permissionMode: "bypass" };
        } else if (key === "off" || key === "default") {
            patch = { permissionMode: "default" };
        } else {
            return {
                kind: "error",
                message: `/bypass: unknown arg '${arg}'. Use '/bypass' to enable or '/bypass off' to disable.`,
            };
        }
        const result = await updateRuntime(ctx, patch);
        if (result.ok === false) return runtimeError(result.error);
        return {
            kind: "ok",
            message: `permission mode set to ${result.updated.permissionMode} (applies to next turn)`,
        };
    },
};

// ── /plan ─────────────────────────────────────────────────────────────

const planCommand: SlashCommand = {
    name: "plan",
    category: "runtime",
    description: "Switch to plan mode (no tool execution)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const result = await updateRuntime(ctx, { permissionMode: "plan" });
        if (result.ok === false) return runtimeError(result.error);
        return { kind: "ok", message: "permission mode set to plan (applies to next turn)" };
    },
};

// ── /runtime ──────────────────────────────────────────────────────────

const runtimeCommand: SlashCommand = {
    name: "runtime",
    category: "runtime",
    description: "Show current runtime config (permission / model / effort)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const r = getRuntimeConfig(ctx.block()?.meta);
        const parts = [
            `permission: ${r.permissionMode}`,
            `model: ${r.model}`,
            `effort: ${r.effort}`,
        ];
        return { kind: "ok", message: `runtime config — ${parts.join(" · ")}` };
    },
};

export const RUNTIME_COMMANDS: SlashCommand[] = [
    modelCommand,
    effortCommand,
    permissionModeCommand,
    bypassCommand,
    planCommand,
    runtimeCommand,
];
