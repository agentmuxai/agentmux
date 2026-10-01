// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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
