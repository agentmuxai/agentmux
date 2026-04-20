// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Builds CLI args from provider base args + runtime overrides.
 *
 * Called before each turn so the user can change permission mode, model,
 * and effort level between messages without restarting the session.
 */

import type { AgentRuntimeConfig, PermissionMode } from "./types";
import { DEFAULT_RUNTIME_CONFIG } from "./types";

/**
 * Permission mode → CLI flags mapping.
 * "bypass" uses the legacy flag; all others use --permission-mode.
 */
const PERMISSION_FLAGS: Record<PermissionMode, string[]> = {
    bypass: ["--dangerously-skip-permissions"],
    auto: ["--permission-mode", "auto"],
    acceptEdits: ["--permission-mode", "acceptEdits"],
    plan: ["--permission-mode", "plan"],
    default: ["--permission-mode", "default"],
};

/** Flags that must be removed before applying permission mode. */
const PERMISSION_STRIP = new Set([
    "--dangerously-skip-permissions",
    "--permission-mode",
    "--yolo",
]);

/**
 * Build final CLI args from base provider args and runtime config.
 *
 * The base args come from ProviderDefinition.launchArgs (e.g.
 * ["-p", "--output-format", "stream-json", "--verbose", ...]).
 * Runtime overrides are applied on top, replacing conflicting flags.
 */
export function buildRuntimeArgs(
    baseLaunchArgs: string[],
    runtime: AgentRuntimeConfig | null | undefined,
    providerId?: string,
): string[] {
    const config = runtime ?? DEFAULT_RUNTIME_CONFIG;
    const args: string[] = [];

    // Copy base args, stripping flags we'll re-add from runtime config
    let i = 0;
    while (i < baseLaunchArgs.length) {
        const arg = baseLaunchArgs[i];
        if (PERMISSION_STRIP.has(arg)) {
            // Skip this flag. If it takes a value (--permission-mode <val>), skip that too.
            if (arg === "--permission-mode" && i + 1 < baseLaunchArgs.length) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if (arg === "--model" && i + 1 < baseLaunchArgs.length) {
            i += 2; // strip existing --model <val>
            continue;
        }
        if (arg === "--effort" && i + 1 < baseLaunchArgs.length) {
            i += 2; // strip existing --effort <val>
            continue;
        }
        args.push(arg);
        i++;
    }

    // Apply permission mode
    if (providerId === "kimi" || providerId === "gemini") {
        // Kimi and Gemini only support --yolo (bypass) vs no flag (default)
        if (config.permissionMode !== "default") {
            args.push("--yolo");
        }
    } else {
        const permFlags = PERMISSION_FLAGS[config.permissionMode] ?? PERMISSION_FLAGS.bypass;
        args.push(...permFlags);
    }

    // Apply model and effort for all providers except Kimi, which does not
    // support the --effort flag and uses different --model values.
    if (providerId !== "kimi") {
        args.push("--model", config.model);
        args.push("--effort", config.effort);
    }

    return args;
}

/**
 * Read AgentRuntimeConfig from block metadata, falling back to defaults.
 */
export function getRuntimeConfig(blockMeta: Record<string, any> | undefined): AgentRuntimeConfig {
    const raw = blockMeta?.["agent:runtime"];
    if (!raw || typeof raw !== "object") return { ...DEFAULT_RUNTIME_CONFIG };
    return {
        permissionMode: raw.permissionMode ?? DEFAULT_RUNTIME_CONFIG.permissionMode,
        model: raw.model ?? DEFAULT_RUNTIME_CONFIG.model,
        effort: raw.effort ?? DEFAULT_RUNTIME_CONFIG.effort,
    };
}
