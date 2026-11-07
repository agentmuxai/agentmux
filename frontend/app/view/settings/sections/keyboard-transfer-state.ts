// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import and export in Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §9):
// the format is the `keybindings` array as it is in settings.json, so there is no second file to keep. These are plain
// functions: what an import would change, and the setting once it is merged or replaces the old one.

import { entryIssue } from "@/app/keybindings/registry";

/** What a pasted text holds: the entries, or why it cannot be read. */
export type Parsed = { entries: unknown[] } | { error: string };

/** The `keybindings` array as text, for export. */
export const exportText = (entries: unknown[]): string => JSON.stringify(entries, null, 2);

/** Reads pasted text as an array of entries, or as a settings object holding a `keybindings` array. */
export function parseImport(text: string): Parsed {
    if (!text.trim()) return { error: "Paste the exported keybindings." };
    let value: unknown;
    try {
        value = JSON.parse(text);
    } catch (err) {
        return { error: `That is not valid JSON: ${(err as Error).message}` };
    }
    const entries = Array.isArray(value) ? value : (value as { keybindings?: unknown } | null)?.keybindings;
    if (!Array.isArray(entries)) return { error: 'Expected an array of entries, or settings with a "keybindings" array.' };
    return { entries };
}

export interface ImportPreview {
    /** The entries the registry would apply: what is imported. */
    usable: unknown[];
    /** The ones it would skip, with the reason: they are not imported. */
    unusable: { index: number; message: string }[];
    /** Usable entries that the setting already has, word for word. */
    already: number;
    /** Commands the import gives keys, where the setting already has keys of its own that differ (merging replaces them). */
    replaced: number;
}

/** The group an entry belongs to when merging: one command on one platform ("-command" is the same command). */
function groupOf(entry: unknown): string {
    const e = entry as { command: string; platform?: string };
    return `${e.command.replace(/^-/, "")}|${e.platform ?? "*"}`;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What importing `incoming` into `current` would do. */
export function previewImport(current: unknown[], incoming: unknown[]): ImportPreview {
    const usable: unknown[] = [];
    const unusable: ImportPreview["unusable"] = [];
    incoming.forEach((e, index) => {
        const message = entryIssue(e);
        if (message == null) usable.push(e);
        else unusable.push({ index, message });
    });
    const have = (e: unknown) => current.some((c) => same(c, e));
    const touched = new Set(usable.map(groupOf));
    const replaced = [...touched].filter((g) => {
        const mine = current.filter((c) => entryIssue(c) == null && groupOf(c) === g);
        const theirs = usable.filter((u) => groupOf(u) === g);
        return mine.length > 0 && !(mine.length === theirs.length && theirs.every(have));
    }).length;
    return { usable, unusable, already: usable.filter(have).length, replaced };
}

/** The setting with the commands the import gives keys to taken from the import: the entries the setting has for them
 *  (on that platform) go, the imported ones are added, every other entry stays as it is. */
export function mergeImport(current: unknown[], usable: unknown[]): unknown[] {
    const touched = new Set(usable.map(groupOf));
    const keep = current.filter((c) => entryIssue(c) != null || !touched.has(groupOf(c)));
    return [...keep, ...usable];
}

/** One sentence on what the import would do. */
export function previewText(p: ImportPreview): string {
    const parts = [`${p.usable.length} ${p.usable.length === 1 ? "entry" : "entries"} to import`];
    if (p.already > 0) parts.push(`${p.already} already in your setting`);
    if (p.replaced > 0) parts.push(`${p.replaced} ${p.replaced === 1 ? "command" : "commands"} with keys of your own that merging replaces`);
    if (p.unusable.length > 0) parts.push(`${p.unusable.length} not usable, skipped`);
    return `${parts.join("; ")}.`;
}
