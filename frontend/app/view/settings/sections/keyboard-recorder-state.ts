// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Recording a shortcut in Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md
// §4.5, §5): what a recorded key would do to the `keybindings` setting, what it collides with, and whether it can
// be saved. Plain functions over `planChange`, so the view holds no rule of its own.

import { DEFAULT_KEYBINDINGS } from "@/app/keybindings/defaults";
import { choicesOf, isPlainKey, PLAIN_KEY_GUARD, planChange, type Conflict, type KeyChoice, type PlannedChange } from "@/app/keybindings/edit";
import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import type { Reserved } from "@/app/keybindings/reserved";

/** A shortcut is at most two keys (a chord). */
export const MAX_STEPS = 2;

export { isPlainKey, PLAIN_KEY_GUARD };

export interface WhereChoice {
    /** The `when` the setting gets; "" is everywhere. */
    value: string;
    label: string;
}

const WHERE: WhereChoice[] = [
    { value: "", label: "Everywhere" },
    { value: "!textInputFocus", label: "Not in a text field" },
    { value: "!terminalFocus", label: "Not in a terminal" },
    { value: "terminalFocus && viewType == term", label: "Only in a terminal" },
    { value: "viewType == editor", label: "Only in the editor" },
    { value: "viewType == files", label: "Only in Files" },
    { value: "viewType == browser", label: "Only in a browser pane" },
];

/** Where the command works today: a key recorded for it works there unless the person says otherwise. */
export const ownWhen = (command: string): string => DEFAULT_KEYBINDINGS.find((r) => r.command === command)?.when ?? "";

/** The places a key can be limited to; the command's own, when it is none of them, comes first. */
export function whereChoices(command: string): WhereChoice[] {
    const own = ownWhen(command);
    return WHERE.some((w) => w.value === own) ? WHERE : [{ value: own, label: "As it works today" }, ...WHERE];
}

export interface RecorderOptions {
    /** Add the key to the command's current keys instead of replacing them. */
    keepCurrent: boolean;
    /** The place chosen in the picker ("" is everywhere). */
    where: string;
    /** The person allowed a bare printable key. */
    allowPlain: boolean;
}

export interface Proposal {
    /** The recorded key or chord, as the setting writes it. */
    key: string;
    /** The first step is a bare printable key. */
    plain: boolean;
    /** Plain, and not allowed. */
    refusedPlain: boolean;
    /** What the command ends up with. */
    wanted: KeyChoice[];
    plan: PlannedChange;
    /** Collisions that stop a save. */
    blocking: Conflict[];
    /** Collisions that do not (a pane's own key, and the like). */
    notes: Conflict[];
    blocked: Reserved | null;
    warned: Reserved | null;
}

/** What recording `steps` for `command` would do, or null when nothing is recorded yet. */
export function propose(entries: unknown[], command: string, steps: string[], options: RecorderOptions, platform: KeyPlatform): Proposal | null {
    if (steps.length === 0) return null;
    const key = steps.join(" ");
    const plain = isPlainKey(steps[0], platform);
    const when = plain && options.allowPlain ? PLAIN_KEY_GUARD : options.where || undefined;
    const wanted: KeyChoice[] = [...(options.keepCurrent ? choicesOf(entries, command, platform) : []), { key, ...(when ? { when } : {}) }];
    const plan = planChange(entries, command, wanted, platform);
    const reserved = plan.reserved.map((r) => r.reserved);
    return {
        key,
        plain,
        refusedPlain: plain && !options.allowPlain,
        wanted,
        plan,
        blocking: plan.conflicts.filter((c) => c.blocking),
        notes: plan.conflicts.filter((c) => !c.blocking),
        blocked: reserved.find((r) => r.tier === "blocked") ?? null,
        warned: reserved.find((r) => r.tier === "warned") ?? null,
    };
}

/**
 * What the person can do with the proposal: `save` it; `replace` the keys it collides with (the other commands
 * lose them); `confirm` a key the system often takes, which they must say they want; or `stop` when it cannot
 * be saved.
 */
export type Verdict = "save" | "replace" | "confirm" | "stop";

export function verdictOf(p: Proposal, confirmed: boolean): Verdict {
    if (p.refusedPlain || p.blocked) return "stop";
    if (p.warned && !confirmed) return "confirm";
    return p.blocking.length > 0 ? "replace" : "save";
}

/** The setting once the proposal is saved; `replace` takes the colliding keys from the other commands. */
export function committed(entries: unknown[], command: string, p: Proposal, platform: KeyPlatform, replace: boolean): unknown[] {
    return planChange(entries, command, p.wanted, platform, { replace }).entries;
}

/** One collision in words: which command has the key, and in what way it gets in the way. */
export function conflictText(c: Conflict, mine: string, platform: KeyPlatform): string {
    const theirs = formatKey(c.key, platform);
    switch (c.kind) {
        case "same":
            return `${theirs} is already ${c.label}, which works in the same places as ${mine}.`;
        case "shadows":
            return `${c.label} starts with ${theirs}, so this key on its own would hide it.`;
        case "shadowed":
            return `${c.label} uses ${theirs} on its own, so this two-step shortcut could never start.`;
        case "pane":
            return `${c.label} uses ${theirs} in its own pane and takes the key first there; anywhere else this shortcut works.`;
    }
}

/** What a screen reader hears about the recording, in one sentence. */
export function recorderStatus(label: string, p: Proposal | null, verdict: Verdict | null, platform: KeyPlatform): string {
    if (!p || !verdict) return `Press the new shortcut for ${label}. Escape cancels.`;
    const shown = formatKey(p.key, platform);
    if (p.refusedPlain) return `${shown} is a plain key: it would fire while you type. Allow plain keys to use it.`;
    if (p.blocked) return `${shown} cannot be used: ${p.blocked.reason}.`;
    if (verdict === "confirm") return `${shown}: ${p.warned?.reason}. Use it anyway to save it.`;
    if (verdict === "replace") return `${shown} is taken by ${p.blocking.map((c) => c.label).join(", ")}. Replace it, or choose where this shortcut works.`;
    return `${shown}. Enter saves, or press another key to make it a two-step shortcut.`;
}
