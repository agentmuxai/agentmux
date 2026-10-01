// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { parseProviderFlags } from "./launch-args";

/**
 * What a runtime setting does, for which model and provider. One definition for
 * the arg builder, the Runtime menu and the slash commands, so a control is
 * never offered for something that is not applied (or applied for something
 * that is not offered).
 *
 * srv decides the same thing for the same reason when it seeds or fills
 * `--effort` (`model_takes_effort` in crates/srv/src/backend/agent_runtime.rs).
 */

/**
 * Whether `--effort` is passed for `model`. Haiku 4.5 rejects it (HTTP 400 on
 * every turn), so a Haiku pane gets none. Matches the model id as well as the
 * alias: this used to be `model === "haiku"`, so a concrete Haiku id such as
 * `claude-haiku-4-5-20251001` still got the flag and failed.
 */
export function modelTakesEffort(model: string): boolean {
    return !/haiku/i.test(model);
}

/**
 * Whether the effort setting changes anything for this pane: only Claude takes
 * `--effort`, and not on Haiku.
 */
export function effortApplies(providerId: string | undefined, model: string): boolean {
    return (!providerId || providerId === "claude") && modelTakesEffort(model);
}

/** Why effort does nothing for this pane, for a message to the user; null when it applies. */
export function effortNotUsedReason(providerId: string | undefined, model: string): string | null {
    if (effortApplies(providerId, model)) return null;
    if (providerId && providerId !== "claude") return `${providerId} has no effort setting`;
    return "Haiku does not use it";
}

/**
 * The model a list of CLI flags selects, if it selects one: the last
 * `--model X`, `-m X` or `--model=X`.
 */
export function modelFromFlags(flags: readonly string[]): string | undefined {
    let model: string | undefined;
    for (let i = 0; i < flags.length; i++) {
        const f = flags[i];
        if ((f === "--model" || f === "-m") && flags[i + 1] !== undefined) {
            model = flags[i + 1];
            i++;
        } else if (f.startsWith("--model=")) {
            model = f.slice("--model=".length);
        }
    }
    return model;
}

/**
 * The model the process actually runs. An agent definition's own `provider_flags`
 * are appended after the runtime's flags, so a `--model` among them wins; a
 * decision that depends on the model (does `--effort` apply?) must be made on
 * THIS, not on the runtime's selection, or a definition that picks Haiku gets
 * `--effort` with the Haiku it overrode to — HTTP 400 every turn
 * (Codex P1 on #4152).
 */
export function effectiveModel(runtimeModel: string, providerFlags: unknown): string {
    return modelFromFlags(parseProviderFlags(providerFlags)) ?? runtimeModel;
}

/** The value of a flag list's `--effort`, if it has one; the last wins. */
export function effortFromFlags(flags: readonly string[]): string | undefined {
    let effort: string | undefined;
    for (let i = 0; i < flags.length; i++) {
        const f = flags[i];
        if (f === "--effort" && flags[i + 1] !== undefined) {
            effort = flags[i + 1];
            i++;
        } else if (f.startsWith("--effort=")) {
            effort = f.slice("--effort=".length);
        }
    }
    return effort;
}

/**
 * The permission mode a flag list selects, if it selects one: the last of
 * `--permission-mode X` (also `=X`), `--dangerously-skip-permissions` and
 * `--yolo` (both are "bypass").
 */
export function permissionModeFromFlags(flags: readonly string[]): string | undefined {
    let mode: string | undefined;
    for (let i = 0; i < flags.length; i++) {
        const f = flags[i];
        if (f === "--permission-mode" && flags[i + 1] !== undefined) {
            mode = flags[i + 1];
            i++;
        } else if (f.startsWith("--permission-mode=")) {
            mode = f.slice("--permission-mode=".length);
        } else if (f === "--dangerously-skip-permissions" || f === "--yolo") {
            mode = "bypass";
        }
    }
    return mode;
}

/**
 * The model, effort and permission mode the process actually runs: the agent
 * definition's own flags (appended after the runtime's) win over the stored
 * selection. What the menu and `/runtime` should SHOW, so they never claim a
 * selection the definition overrides.
 */
export function effectiveRuntime<T extends { model: string; effort: string; permissionMode?: string }>(
    runtime: T,
    providerFlags: unknown,
): T {
    const flags = parseProviderFlags(providerFlags);
    return {
        ...runtime,
        model: modelFromFlags(flags) ?? runtime.model,
        effort: effortFromFlags(flags) ?? runtime.effort,
        ...(runtime.permissionMode !== undefined
            ? { permissionMode: permissionModeFromFlags(flags) ?? runtime.permissionMode }
            : {}),
    };
}
