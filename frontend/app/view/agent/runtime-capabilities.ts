// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isWindows } from "@/util/platformutil";
import { parseProviderFlags } from "./launch-args";
import { getProvider } from "./providers";
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

/** Claude's `--effort` levels, in order. */
export const CLAUDE_EFFORTS: readonly string[] = ["low", "medium", "high", "xhigh", "max"];

/** A Codex model as its account's list describes it, or undefined while that list is unknown. */
function codexModel(model: string) {
    return getProvider("codex")?.models?.find((m) => m.value === model);
}

/**
 * The effort levels this pane's model takes, in order; empty when the effort
 * setting does nothing. Claude: its five, none on Haiku. Codex: the model's
 * own, from its account's list (`low` … `ultra`, different per model; none
 * until that list is known). Others: none.
 */
export function effortLevels(providerId: string | undefined, model: string): readonly string[] {
    if (!providerId || providerId === "claude") return modelTakesEffort(model) ? CLAUDE_EFFORTS : [];
    if (providerId === "codex") return codexModel(model)?.efforts ?? [];
    return [];
}

/**
 * Whether the effort setting changes anything for this pane: Claude's
 * `--effort` (not on Haiku), and Codex's `model_reasoning_effort` for a model
 * whose levels its account lists.
 */
export function effortApplies(providerId: string | undefined, model: string): boolean {
    return effortLevels(providerId, model).length > 0;
}

/**
 * The effort a Codex turn runs at: the selection when the model takes it, or
 * the model's own default (nothing is passed then, and the CLI uses that).
 * Selections are shared with Claude's levels, so a pane can hold one this
 * model doesn't take ("max" on a model that stops at "xhigh").
 */
export function codexEffort(model: string, selected: string): { effort: string | undefined; passed: boolean } {
    const levels = effortLevels("codex", model);
    if (levels.includes(selected)) return { effort: selected, passed: true };
    return { effort: codexModel(model)?.defaultEffort, passed: false };
}

/** Why effort does nothing for this pane, for a message to the user; null when it applies. */
export function effortNotUsedReason(providerId: string | undefined, model: string): string | null {
    if (effortApplies(providerId, model)) return null;
    if (providerId === "codex") {
        return codexModel(model) ? "this model takes no effort setting" : "the account's models aren't known yet";
    }
    if (providerId && providerId !== "claude") return `${providerId} has no effort setting`;
    return "Haiku does not use it";
}

/**
 * Codex's sandbox for a permission mode (`-c sandbox_mode=…`; `codex exec
 * resume` has no `--sandbox` flag, so the config override serves both).
 * Bypass is the full-access default its base args already carry. The
 * Claude-only modes a pane may still hold fall to the nearest one: Auto
 * writes like Accept Edits, Default can't ask anyone in `exec`, so it reads only.
 *
 * On Windows, Codex has no workspace-write sandbox yet: observed with
 * codex-cli 0.154.0 on 2026-10-22, `sandbox_mode=workspace-write` blocked a
 * write in the working directory, in a git repo too, exactly as `read-only`
 * did (its Windows sandbox service is "under development"). So there the
 * writing modes are Read only, and the menu says so rather than offering a
 * Workspace write that doesn't write.
 */
export function codexSandbox(mode: PermissionMode, onWindows: boolean = isWindows()): "workspace-write" | "read-only" | null {
    switch (mode) {
        case "bypass":
            return null;
        case "acceptEdits":
        case "auto":
            return onWindows ? "read-only" : "workspace-write";
        case "plan":
        case "default":
            return "read-only";
    }
}

interface CodexMode {
    value: PermissionMode;
    label: string;
    menuLabel: string;
    note: string;
}

const CODEX_FULL_ACCESS: CodexMode = { value: "bypass", label: "Full access", menuLabel: "Full access", note: "no sandbox, no prompts" };
const CODEX_WORKSPACE_WRITE: CodexMode = {
    value: "acceptEdits",
    label: "Workspace write",
    menuLabel: "Workspace write",
    note: "sandboxed: edits in the workspace only",
};
const CODEX_READ_ONLY: CodexMode = { value: "plan", label: "Read only", menuLabel: "Read only", note: "sandboxed: no edits or writes" };

/** The modes a Codex pane offers, as Codex names them: no Workspace write on Windows (codexSandbox). */
export function codexModes(onWindows: boolean = isWindows()): readonly CodexMode[] {
    return onWindows ? [CODEX_FULL_ACCESS, CODEX_READ_ONLY] : [CODEX_FULL_ACCESS, CODEX_WORKSPACE_WRITE, CODEX_READ_ONLY];
}

/** The Codex mode a stored permission mode runs as. */
export function codexModeFor(mode: PermissionMode, onWindows: boolean = isWindows()): CodexMode {
    const sandbox = codexSandbox(mode, onWindows);
    return codexModes(onWindows).find((m) => codexSandbox(m.value, onWindows) === sandbox) ?? CODEX_FULL_ACCESS;
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
 * (the cli-probe harness, docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §7).
 * That is the CLAUDE CLI only: no other provider's CLI was run, so for them this
 * makes no claim about what a mode does (`providerId`).
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
    providerId = "claude",
): { label: string; note?: string; detail?: string } {
    if (providerId === "codex") {
        // Codex's own sandbox modes (codexSandbox): a Claude-only mode shows as the one it runs.
        const m = codexModeFor(mode);
        return { label: m.menuLabel, note: m.note };
    }
    if (providerId !== "claude") {
        // Gemini, Kimi and Qwen get `--yolo` for every non-default mode and Codex
        // gets no permission flag at all (buildRuntimeArgs.ts), and none of their
        // CLIs were observed: no promise either way.
        const names: Record<PermissionMode, string> = {
            bypass: "Bypass (no prompts)",
            auto: "Auto",
            acceptEdits: "Accept Edits",
            plan: "Plan",
            default: "Default",
        };
        return { label: names[mode] };
    }
    if (!autoAnswersPrompts) {
        // Nothing to ask, so the CLI refuses what the mode does not allow.
        switch (mode) {
            case "bypass":
                return { label: "Bypass (no prompts)" };
            case "default":
                return {
                    label: "Default",
                    note: "unapproved writes and commands refused",
                    detail: "unapproved writes and commands are refused (there is no one to ask)",
                };
            case "acceptEdits":
                return {
                    label: "Accept Edits",
                    note: "edits allowed; other commands refused",
                    detail: "edits are allowed; other unapproved commands are refused",
                };
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
                note: "every ask auto-approved, like Bypass",
                detail: "writes and commands are asked about, and every ask is allowed automatically — same as Bypass for now",
            };
        case "acceptEdits":
            return {
                label: "Accept Edits",
                note: "other asks auto-approved, like Bypass",
                detail: "edits are not asked about; any other ask is allowed automatically — same as Bypass for now",
            };
        case "auto":
            return {
                label: "Auto",
                note: "classifier asks; all auto-approved",
                detail: "the CLI's classifier decides what to ask; every ask is allowed automatically",
            };
        case "plan":
            return {
                label: "Plan",
                note: "does NOT stop edits; plan auto-approved",
                detail: "writes are asked about and allowed automatically, so this does NOT stop edits; the plan is approved automatically",
            };
    }
}
