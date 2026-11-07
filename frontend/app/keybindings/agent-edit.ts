// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent changing the person's shortcuts (MuxAPI `UI` actions `bind` and `reset`,
// docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §10, D6): the same edit the Settings view makes, through
// `planChange`, with the protections an agent gets. It never replaces another command's key and never takes a key the
// system takes: it returns what is in the way, and the agent asks the person. Pure; the store module writes and tells
// the person.

import { DEFAULT_KEYBINDINGS } from "./defaults";
import { choicesOf, isPlainKey, ownWhen, planChange } from "./edit";
import { formatKey, parseKey, type KeyPlatform } from "./keys";
import { bindingsView, defaultKeysOf, validEntries } from "./registry";
import { reservedFor } from "./reserved";

export interface AgentSetRequest {
    command: string;
    /** The keys the command ends with, each a key or a two-step chord ("ctrl+k ctrl+s"); empty unbinds it. */
    bindings: string[];
    /** Where they work; absent, where the command works today. */
    when?: string;
}

export interface AgentResetRequest {
    /** One command back on its defaults. */
    command?: string;
    /** Every command back on its defaults. */
    all?: boolean;
}

/** A command that has the key an agent asked for. */
export interface Taken {
    command: string;
    label: string;
    key: string;
}

export type AgentEdit =
    | {
          changed: true;
          /** The whole new setting. */
          entries: unknown[];
          /** How many commands differ from before. */
          count: number;
          /** One line for the person: what changed. */
          summary: string;
          /** The keys the command has now (empty for a reset of everything). */
          keys: string[];
      }
    | { changed: false; reason: string; taken?: Taken[] };

const refuse = (reason: string, taken?: Taken[]): AgentEdit => ({ changed: false, reason, ...(taken ? { taken } : {}) });

const isTableCommand = (command: string): boolean => DEFAULT_KEYBINDINGS.some((r) => r.command === command);
const labelOf = (command: string): string => DEFAULT_KEYBINDINGS.find((r) => r.command === command)?.label ?? command;
const same = (a: unknown[], b: unknown[]): boolean => JSON.stringify(a) === JSON.stringify(b);

/** The parse error of a key, or null: a key is steps joined by one space, at most two. */
function keyProblem(key: string, platform: KeyPlatform): string | null {
    const steps = key.trim().split(/\s+/);
    if (!key.trim()) return "a key is empty";
    if (steps.length > 2) return `"${key}": a shortcut is at most two keys`;
    try {
        steps.forEach((s) => parseKey(s, platform));
    } catch (err) {
        return (err as Error).message;
    }
    return null;
}

/** Why the system keeps `key` from being an agent's to bind, or null. A key the desktop only often takes is the
 *  person's to confirm in Settings ("Use it anyway"), so an agent is told to ask. */
function reservedProblem(key: string, platform: KeyPlatform): string | null {
    const r = reservedFor(key, platform);
    if (!r) return null;
    const shown = formatKey(key, platform);
    return r.tier === "blocked" ? `${shown} cannot be used: ${r.reason}` : `${shown}: ${r.reason}. Ask the person to set it in Settings, where they can confirm it`;
}

/** A key with no Ctrl, Alt or Meta that the person types or edits with: a letter, digit or punctuation key, or a named
 *  one (Enter, Backspace, Tab, the arrows, Home, End...) but a function key. */
function typesOrEdits(step: string, platform: KeyPlatform): boolean {
    if (isPlainKey(step, platform)) return true;
    const k = parseKey(step, platform);
    return k.named != null && !k.ctrl && !k.alt && !k.meta && !/^F\d{1,2}$/.test(k.named);
}

/** Such a key would run the command in every text field and terminal unless it is limited to where the person is
 *  not typing: the Settings recorder asks the person to allow it, so an agent leaves it to them. */
function plainProblem(key: string, platform: KeyPlatform): string | null {
    if (!typesOrEdits(key.split(" ")[0], platform)) return null;
    return `${formatKey(key, platform)} is a key the person types or edits with. Ask the person to set it in Settings, where they can allow plain keys`;
}

/** What `req` does to `entries`, or why not. */
export function planAgentSet(entries: unknown[], req: AgentSetRequest, platform: KeyPlatform): AgentEdit {
    const command = req.command?.trim();
    if (!command || !isTableCommand(command)) return refuse(`unknown command "${req.command ?? ""}": the \`shortcuts\` action lists them`);
    if (!Array.isArray(req.bindings)) return refuse("`bindings` must be a list of keys (empty to unbind the command)");
    const keys = req.bindings.map((k) => String(k).trim().split(/\s+/).join(" "));
    for (const key of keys) {
        const bad = keyProblem(key, platform) ?? reservedProblem(key, platform) ?? plainProblem(key, platform);
        if (bad) return refuse(bad);
    }
    // A blank `when` is no `when` given: the key works where the command works today.
    const when = req.when?.trim() || ownWhen(command);
    const plan = planChange(entries, command, keys.map((key) => ({ key, ...(when ? { when } : {}) })), platform);
    const written = plan.entries.filter((e) => !entries.includes(e));
    if (written.some((e) => validEntries([e]).length === 0)) {
        return refuse(`"${req.when}" is not a place a shortcut can work: use textInputFocus, terminalFocus, docTabsHost or viewType == <pane>, each optionally with !, joined with &&`);
    }
    const blocking = plan.conflicts.filter((c) => c.blocking);
    if (blocking.length > 0) {
        const taken = blocking.map((c) => ({ command: c.command, label: c.label, key: c.key }));
        return refuse(`${blocking.map((c) => `${formatKey(c.key, platform)} is ${c.label}`).join("; ")}; an agent does not take a key from another command (ask the person, or use another key)`, taken);
    }
    if (same(plan.entries, entries)) return refuse(`${labelOf(command)} already has ${keys.length > 0 ? keys.map((k) => formatKey(k, platform)).join(", ") : "no key"}`);
    const now = choicesOf(plan.entries, command, platform).map((c) => c.key);
    return { changed: true, entries: plan.entries, count: 1, keys: now, summary: `${labelOf(command)}: ${now.length > 0 ? now.map((k) => formatKey(k, platform)).join(", ") : "no key"}` };
}

/** What `req` does to `entries`, or why not. */
export function planAgentReset(entries: unknown[], req: AgentResetRequest, platform: KeyPlatform): AgentEdit {
    if (req.all) {
        const changed = bindingsView(platform).filter((v) => !v.devOnly && v.changed).length;
        if (entries.length === 0) return refuse("no shortcut is changed");
        // Entries that did not apply (an unknown command, a bad key) are cleared too, though no command differs.
        const summary = changed > 0
            ? `all shortcuts back on their defaults (${changed} ${changed === 1 ? "command" : "commands"})`
            : `the shortcuts setting cleared (${entries.length} ${entries.length === 1 ? "entry" : "entries"} that did not apply)`;
        return { changed: true, entries: [], count: changed || entries.length, keys: [], summary };
    }
    const command = req.command?.trim();
    if (!command || !isTableCommand(command)) return refuse(`unknown command "${req.command ?? ""}": give a command, or \`all\` to reset every shortcut`);
    const plan = planChange(entries, command, defaultKeysOf(command, platform), platform);
    if (same(plan.entries, entries)) return refuse(`${labelOf(command)} is already on its defaults`);
    const now = choicesOf(plan.entries, command, platform).map((c) => c.key);
    return { changed: true, entries: plan.entries, count: 1, keys: now, summary: `${labelOf(command)} back on its default: ${now.map((k) => formatKey(k, platform)).join(", ") || "no key"}` };
}
