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
import { getProvider } from "./providers";

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

// Default codex model. The Claude-named `ModelChoice` (opus/sonnet/haiku) does
// not apply to codex, and codex 0.116.0's baked default (gpt-5.3-codex) was
// rejected for ChatGPT-account auth. gpt-5.5 remains valid — re-verified
// 2026-09-06 against OpenAI's own Codex model docs when the CLI pin bumped to
// 0.153.4 (SPEC_PROVIDER_CLI_VERSION_UPGRADE_2026_09_06.md): gpt-5.5 is
// explicitly still supported for ChatGPT sign-in auth, described there as the
// "previous-generation flagship" (gpt-6-astra is newer but in a staged,
// limited-org rollout as of that date, so gpt-5.5 stays the safer default).
// Per-provider model selection is a follow-up — see
// docs/specs/SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md
// (re-verify ChatGPT-account availability when bumping the codex CLI pin).
const CODEX_DEFAULT_MODEL = "gpt-5.5";

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

    // Apply permission mode.
    // Codex takes no appended permission flag: its bypass
    // (--dangerously-bypass-approvals-and-sandbox) is baked into its base args,
    // `codex exec` has no --permission-mode, and its prompt positional `-` must
    // stay last (anything appended after it is parsed as a stray arg).
    if (providerId === "kimi" || providerId === "gemini" || providerId === "qwen") {
        // Kimi, Gemini, and Qwen (a Gemini-CLI fork) only support --yolo
        // (bypass) vs no flag (default) — not --permission-mode / --dangerously-skip-permissions
        if (config.permissionMode !== "default") {
            args.push("--yolo");
        }
    } else if (providerId !== "codex") {
        const permFlags = PERMISSION_FLAGS[config.permissionMode] ?? PERMISSION_FLAGS.bypass;
        args.push(...permFlags);
    }

    // --model: claude only. ModelChoice values (opus/sonnet/haiku) are Claude
    // model names; codex (handled below) and gemini use their own model
    // namespaces, so a Claude name is rejected. gemini falls back to its CLI
    // default until a per-provider model list lands. --effort: claude only.
    const supportsModel = !providerId || providerId === "claude";
    if (supportsModel) {
        args.push("--model", config.model);
    }
    // --effort: claude only, and NOT on Haiku — `--effort` 400s on Haiku 4.5
    // (effort is supported on Opus/Sonnet only). Skip it so a `haiku` pane
    // doesn't error out on every turn.
    if ((!providerId || providerId === "claude") && config.model !== "haiku") {
        args.push("--effort", config.effort);
    }

    // Codex uses a gpt-5.x model, and the flag must precede its trailing `-`
    // prompt positional. Use the user-picked model when it's a valid codex
    // model; otherwise the provider default. (A pane carried over from before
    // per-provider models may still have a Claude model like "opus" stored —
    // never pass that to codex.)
    if (providerId === "codex") {
        const codexModels = getProvider("codex")?.models ?? [];
        const picked = codexModels.find((m) => m.value === config.model)?.value;
        const fallback = codexModels.find((m) => m.default)?.value ?? CODEX_DEFAULT_MODEL;
        const promptPositional = args[args.length - 1] === "-" ? args.pop()! : null;
        args.push("--model", picked ?? fallback);
        if (promptPositional !== null) args.push(promptPositional);
    }

    return args;
}

/**
 * Whether `buildRuntimeArgs` actually applies `AgentRuntimeConfig.model`
 * to this provider's launch args at all (via `--model`, through either
 * the plain claude branch above or codex's own dedicated branch below).
 * The single source of truth a model PICKER (AgentRuntimeDropup,
 * AgentCreateFromTemplateModal) should gate on before offering a choice
 * — otherwise a provider with a `models` list in the catalog (e.g.
 * antigravity) but no `--model` wiring here lets the user pick a model
 * that's silently discarded at launch (ReAgent P2 on PR #2618).
 *
 * Kept in this file specifically (not providers/catalog.ts) so it can
 * never drift from the actual arg-building logic above — a provider
 * added to one without the other is exactly the bug class this function
 * exists to close.
 */
export function providerSupportsModelFlag(providerId: string | undefined): boolean {
    return !providerId || providerId === "claude" || providerId === "codex";
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
