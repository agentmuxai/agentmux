// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4): what the list
// shows for a search and the filters, and the changes to the `keybindings` setting its buttons make. Plain
// functions over `bindingsView`, so the view holds no rule of its own.

import { createSignal } from "solid-js";

import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import { planChange } from "@/app/keybindings/edit";
import { defaultKeysOf, type CommandBindings, type KeybindingIssue } from "@/app/keybindings/registry";
import type { KeyCategory } from "@/app/keybindings/defaults";
import { keyLabelWords } from "@/app/keybindings/help";
import { matchesEveryWord } from "@/app/util/fuzzysearch";

export type CategoryFilter = KeyCategory | "all";

export interface ShortcutFilters {
    query: string;
    category: CategoryFilter;
    changedOnly: boolean;
}

/** A line of the list: one command, or the commands of a `helpGroup` ("Go to tab 1–8") shown as one. */
export interface ShortcutRow {
    id: string;
    label: string;
    /** More than one for a group; the first is the one the line's buttons act on when it is a single command. */
    commands: CommandBindings[];
}

export interface CategoryBlock {
    category: KeyCategory;
    rows: ShortcutRow[];
}

const CATEGORY_ORDER: KeyCategory[] = ["General", "Tabs & windows", "Panes", "Find & zoom", "Terminal", "Documents", "Editor", "Files"];

/** The categories that have at least one command a person can rebind, in the order Help shows them. */
export function categoriesOf(view: CommandBindings[]): KeyCategory[] {
    const present = new Set(view.filter((v) => !v.devOnly).map((v) => v.category));
    return CATEGORY_ORDER.filter((c) => present.has(c));
}

const wordsOf = (v: CommandBindings, platform: KeyPlatform): string[] => [
    v.label,
    v.helpGroup ?? "",
    v.command,
    v.category,
    ...v.keys.flatMap((k) => {
        const label = formatKey(k, platform);
        return [label, keyLabelWords(label), k];
    }),
];

/**
 * The list for a search and the filters, grouped by category in Help's order. A group is kept when any of its
 * commands matches, and shows only those that do. Developer-only commands are left out, as in Help.
 */
export function shortcutBlocks(view: CommandBindings[], filters: ShortcutFilters, platform: KeyPlatform): CategoryBlock[] {
    const blocks = new Map<KeyCategory, Map<string, ShortcutRow>>();
    for (const v of view) {
        if (v.devOnly) continue;
        if (filters.category !== "all" && v.category !== filters.category) continue;
        if (filters.changedOnly && !v.changed) continue;
        if (!matchesEveryWord(filters.query, ...wordsOf(v, platform))) continue;
        const rows = blocks.get(v.category) ?? new Map<string, ShortcutRow>();
        blocks.set(v.category, rows);
        const id = v.helpGroup ?? v.command;
        const row = rows.get(id) ?? { id, label: v.helpGroup ?? v.label, commands: [] };
        row.commands.push(v);
        rows.set(id, row);
    }
    return CATEGORY_ORDER.filter((c) => blocks.has(c)).map((category) => ({ category, rows: [...blocks.get(category)!.values()] }));
}

/** How many commands differ from their defaults. */
export const changedCount = (view: CommandBindings[]): number => view.filter((v) => !v.devOnly && v.changed).length;

/** The skipped entries that name no command the list shows: they go under "Other entries". */
export function otherEntries(issues: KeybindingIssue[], view: CommandBindings[]): KeybindingIssue[] {
    const shown = new Set(view.map((v) => v.command));
    return issues.filter((i) => !shown.has(i.command));
}

/** The setting with `command` back on its defaults: every entry of its own for this platform is dropped. */
export function resetCommand(entries: unknown[], command: string, platform: KeyPlatform): unknown[] {
    return planChange(entries, command, defaultKeysOf(command, platform), platform).entries;
}

/** The setting with `command` left without a key, written as the one bare `-command` that keeps it off. */
export function unbindCommand(entries: unknown[], command: string, platform: KeyPlatform): unknown[] {
    return planChange(entries, command, [], platform).entries;
}

/** The setting without its `index`th entry (a skipped one the person chose to remove). */
export const removeEntry = (entries: unknown[], index: number): unknown[] => entries.filter((_, i) => i !== index);

/** The search box, the category choice and the "changed only" switch. */
export function createKeyboardFilters() {
    const [query, setQuery] = createSignal("");
    const [category, setCategory] = createSignal<CategoryFilter>("all");
    const [changedOnly, setChangedOnly] = createSignal(false);
    return { query, setQuery, category, setCategory, changedOnly, setChangedOnly, filters: (): ShortcutFilters => ({ query: query(), category: category(), changedOnly: changedOnly() }) };
}

export type KeyboardFilters = ReturnType<typeof createKeyboardFilters>;
