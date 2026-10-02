// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { parseProviderFlags } from "./launch-args";
import type { PermissionMode } from "./types";

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
 * Whether `--effort` is passed for `model`. Haiku takes none: the pinned CLI
 * (2.1.285) was observed to DROP `--effort` for Haiku and send no effort at all
 * (`get_settings` reports `effort: null`), so passing it is harmless there. Older
 * CLIs are documented to have forwarded it, and the API answered HTTP 400
 * (docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md), so a Haiku pane is
 * never given one, and the menu says it is not applied. Matches the model id as well as the
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
 * `--effort` with the Haiku it overrode to — a flag Haiku does not take
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

/**
 * What a permission mode actually does, so the menu and `/permission-mode`
 * don't promise more than happens.
 *
 * OBSERVED, not assumed - by running the pinned CLI (2.1.285) against a fake API
 * (scripts/cli-probe, docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §7):
 *
 * With the permission prompt tool (a persistent agent), the CLI SENDS a
 * `can_use_tool` request for what its mode wants approval for, and srv answers
 * EVERY one "allow" itself (`handle_control_frame`, crates/srv/src/backend/
 * blockcontroller/persistent/input.rs; AskUserQuestion is the exception - it is
 * shown to the user). The CLI asked about: in Default, writes, reads outside the
 * working directory, and shell commands that are not known-safe (`touch`, `curl`;
 * not `echo`); in Accept Edits, the same minus edits; in Plan, WRITES (it did not
 * refuse them) and `ExitPlanMode`. An "allow" lets the write happen. So there,
 * Default and Accept Edits behave like Bypass, and **Plan does not stop edits**.
 *
 * Without it (a container's one-shot run, nothing to ask), the CLI itself refuses:
 * Default refuses unapproved writes and commands ("requested permissions"), and
 * Plan refuses writes ("Cannot write") and blocks commands. There "read-only" is true.
 *
 * Auto was not observed (its classifier needs the real API).
 *
 * `autoAnswersPrompts`: whether this pane's permission requests are answered
 * for the user, i.e. a persistent (control-protocol) launch.
 */
export function permissionModeText(
    mode: PermissionMode,
    autoAnswersPrompts: boolean,
): { label: string; note?: string } {
    if (!autoAnswersPrompts) {
        // Nothing to ask, so the CLI refuses what the mode does not allow.
        switch (mode) {
            case "bypass":
                return { label: "Bypass (no prompts)" };
            case "default":
                return { label: "Default", note: "unapproved writes and commands are refused (there is no one to ask)" };
            case "acceptEdits":
                return { label: "Accept Edits", note: "edits are allowed; other unapproved commands are refused" };
            case "auto":
                return { label: "Auto (AI classifier)" };
            case "plan":
                return { label: "Plan (read-only)" };
        }
    }
    switch (mode) {
        case "bypass":
            return { label: "Bypass (no prompts)" };
        case "default":
            return {
                label: "Default",
                note: "writes and commands are asked about, and every ask is allowed automatically — same as Bypass for now",
            };
        case "acceptEdits":
            return {
                label: "Accept Edits",
                note: "edits are not asked about; any other ask is allowed automatically — same as Bypass for now",
            };
        case "auto":
            return { label: "Auto", note: "the CLI's classifier decides what to ask; every ask is allowed automatically" };
        case "plan":
            return {
                label: "Plan",
                note: "writes are asked about and allowed automatically, so this does NOT stop edits; the plan is approved automatically",
            };
    }
}
