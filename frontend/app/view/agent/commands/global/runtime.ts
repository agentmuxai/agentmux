// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runtime-config slash commands: /model /effort /permission-mode /bypass
 * /plan /runtime. Migrated from the inline switch in
 * useAgentCommands.sendMessage (PR #378) into the registry.
 *
 * Step 1 of specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md —
 * behavior is intentionally identical to #378. Enum/picker work lands
 * in step 2.
 */

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import * as WOS from "@/app/store/wos";
import { getRuntimeConfig } from "../../buildRuntimeArgs";
import type { AgentRuntimeConfig, EffortLevel, ModelChoice, PermissionMode } from "../../types";
import type { SlashCommand, SlashCommandContext, SlashResult } from "../types";

// Alias tables — preserved from PR #378 so `/model claude-sonnet` and
// `/effort med` keep working. Step 2 replaces these with picker enums.
const MODEL_ALIASES: Record<string, ModelChoice> = {
    "opus": "opus",
    "sonnet": "sonnet",
    "haiku": "haiku",
    "claude-opus": "opus",
    "claude-sonnet": "sonnet",
    "claude-haiku": "haiku",
    "default": null,
    "": null,
};

const EFFORT_ALIASES: Record<string, EffortLevel> = {
    "low": "low",
    "medium": "medium",
    "med": "medium",
    "high": "high",
    "max": "max",
    "default": null,
    "": null,
};

const PERMISSION_ALIASES: Record<string, PermissionMode> = {
    "bypass": "bypass",
    "dangerous": "bypass",
    "skip": "bypass",
    "dangerously-skip-permissions": "bypass",
    "auto": "auto",
    "accept": "acceptEdits",
    "acceptedits": "acceptEdits",
    "accept-edits": "acceptEdits",
    "plan": "plan",
    "default": "default",
    "": "default",
};

/**
 * Mutate block.meta["agent:runtime"] with a partial runtime config.
 * Returns the merged config on success, null on RPC failure — callers
 * check and skip the success message if the mutation didn't land.
 */
async function updateRuntime(
    ctx: SlashCommandContext,
    patch: Partial<AgentRuntimeConfig>,
): Promise<AgentRuntimeConfig | null> {
    const current = getRuntimeConfig(ctx.block()?.meta);
    const updated: AgentRuntimeConfig = { ...current, ...patch };
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", ctx.blockId),
            meta: { "agent:runtime": updated },
        });
        return updated;
    } catch (err: any) {
        return null;
    }
}

export const modelCommand: SlashCommand = {
    name: "model",
    category: "runtime",
    description: "Change the active model (applies to next turn)",
    arg: { kind: "freeform", placeholder: "opus | sonnet | haiku | default", required: false },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const key = arg.toLowerCase();
        if (!(key in MODEL_ALIASES)) {
            return {
                kind: "error",
                message: `/model: unknown model '${arg}'. Try: opus | sonnet | haiku | default`,
            };
        }
        const model = MODEL_ALIASES[key];
        const updated = await updateRuntime(ctx, { model });
        if (!updated) {
            return { kind: "error", message: "failed to update runtime config" };
        }
        const label = updated.model ?? "default";
        return { kind: "ok", message: `model set to ${label} (applies to next turn)` };
    },
};

export const effortCommand: SlashCommand = {
    name: "effort",
    category: "runtime",
    description: "Change reasoning effort level (applies to next turn)",
    arg: { kind: "freeform", placeholder: "low | medium | high | max | default", required: false },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const key = arg.toLowerCase();
        if (!(key in EFFORT_ALIASES)) {
            return {
                kind: "error",
                message: `/effort: unknown level '${arg}'. Try: low | medium | high | max | default`,
            };
        }
        const effort = EFFORT_ALIASES[key];
        const updated = await updateRuntime(ctx, { effort });
        if (!updated) {
            return { kind: "error", message: "failed to update runtime config" };
        }
        const label = updated.effort ?? "default";
        return { kind: "ok", message: `effort set to ${label} (applies to next turn)` };
    },
};

export const permissionModeCommand: SlashCommand = {
    name: "permission-mode",
    aliases: ["permission", "perm"],
    category: "runtime",
    description: "Change permission mode (applies to next turn)",
    arg: { kind: "freeform", placeholder: "bypass | auto | accept | plan | default", required: false },
    availability: "any-agent",
    handler: async (ctx, arg): Promise<SlashResult> => {
        const key = arg.toLowerCase();
        if (!(key in PERMISSION_ALIASES)) {
            return {
                kind: "error",
                message: `/permission-mode: unknown mode '${arg}'. Try: bypass | auto | accept | plan | default`,
            };
        }
        const mode = PERMISSION_ALIASES[key];
        const updated = await updateRuntime(ctx, { permissionMode: mode });
        if (!updated) {
            return { kind: "error", message: "failed to update runtime config" };
        }
        return {
            kind: "ok",
            message: `permission mode set to ${updated.permissionMode} (applies to next turn)`,
        };
    },
};

export const bypassCommand: SlashCommand = {
    name: "bypass",
    category: "runtime",
    description: "Enable permission bypass for the next turn (dangerous)",
    // Shortcut: bare `/bypass` enables; `/bypass off` or `/bypass default`
    // reverts. Any other arg is a typo — warn instead of silently enabling.
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
        const updated = await updateRuntime(ctx, patch);
        if (!updated) {
            return { kind: "error", message: "failed to update runtime config" };
        }
        return {
            kind: "ok",
            message: `permission mode set to ${updated.permissionMode} (applies to next turn)`,
        };
    },
};

export const planCommand: SlashCommand = {
    name: "plan",
    category: "runtime",
    description: "Switch to plan mode (no tool execution)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const updated = await updateRuntime(ctx, { permissionMode: "plan" });
        if (!updated) {
            return { kind: "error", message: "failed to update runtime config" };
        }
        return { kind: "ok", message: "permission mode set to plan (applies to next turn)" };
    },
};

export const runtimeCommand: SlashCommand = {
    name: "runtime",
    category: "runtime",
    description: "Show current runtime config (permission / model / effort)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const r = getRuntimeConfig(ctx.block()?.meta);
        const parts = [
            `permission: ${r.permissionMode}`,
            `model: ${r.model ?? "default"}`,
            `effort: ${r.effort ?? "default"}`,
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
