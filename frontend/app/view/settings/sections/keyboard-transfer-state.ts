// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import and export in Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §9):
// the format is the `keybindings` array as it is in settings.json, so there is no second file to keep. These are plain
// functions: what an import would change, and the setting once it is merged or replaces the old one.

import type { KeyPlatform } from "@/app/keybindings/keys";
import { entryIssue } from "@/app/keybindings/registry";
import { reservedFor } from "@/app/keybindings/reserved";

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

type Entry = { command: string; key?: string; platform?: string };

/** The command an entry is about ("-command" unbinds the same command). */
const commandOf = (entry: unknown): string => (entry as Entry).command.replace(/^-/, "");

/** The platforms an entry applies to: its own, or both when it names none. */
const platformsOf = (entry: unknown): string[] => {
    const p = (entry as Entry).platform;
    return p ? [p] : ["mac", "other"];
};

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** Why an entry cannot be imported on `platform`: its key is one the system takes before the app (the recorder refuses
 *  it too). An unbind is harmless. Keys the desktop only often takes are allowed, as the recorder allows them once
 *  confirmed. */
function blockedOn(entry: unknown, platform: string): string | null {
    const { command, key } = entry as Entry;
    if (command.startsWith("-") || !key) return null;
    const r = reservedFor(key, platform as KeyPlatform);
    return r?.tier === "blocked" ? `${key} cannot be used: ${r.reason}` : null;
}

/** The entry to import, or why not. One with no `platform` applies on both, so a key the system takes on only one of
 *  them (⌘ is the Windows key there) is imported for the other, instead of refusing a key that works where the person
 *  is; a key blocked on both is refused. */
function importable(entry: unknown): { entry: unknown } | { why: string } {
    const platform = (entry as Entry).platform;
    const blocked = (platform ? [platform] : ["mac", "other"]).map((p) => [p, blockedOn(entry, p)] as const);
    const refused = blocked.filter(([, why]) => why != null);
    if (refused.length === 0) return { entry };
    if (platform || refused.length === blocked.length) return { why: refused[0][1]! };
    return { entry: { ...(entry as Entry), platform: blocked.find(([, why]) => why == null)![0] } };
}

/** What importing `incoming` into `current` would do. */
export function previewImport(current: unknown[], incoming: unknown[]): ImportPreview {
    const usable: unknown[] = [];
    const unusable: ImportPreview["unusable"] = [];
    incoming.forEach((e, index) => {
        // The reserved-key check only reads an entry the registry accepts.
        const message = entryIssue(e);
        const found = message == null ? importable(e) : { why: message };
        if ("entry" in found) usable.push(found.entry);
        else unusable.push({ index, message: found.why });
    });
    const have = (e: unknown) => current.some((c) => same(c, e));
    const merged = mergeImport(current, usable);
    const valid = (list: unknown[], command: string) => list.filter((c) => entryIssue(c) == null && commandOf(c) === command);
    const commands = [...new Set(usable.map(commandOf))];
    const replaced = commands.filter((command) => {
        const before = valid(current, command);
        const after = valid(merged, command);
        // Replaced is an entry of the setting that the merge removes or changes; one it only adds to is not.
        return !before.every((e) => after.some((a) => same(a, e)));
    }).length;
    return { usable, unusable, already: usable.filter(have).length, replaced };
}

/** The setting with the commands the import gives keys to taken from the import. For each such command the entries the
 *  setting has go where the import covers the platform they apply to, and the imported ones are added; every other
 *  entry stays as it is. An entry with no `platform` applies on both platforms, so one the import covers on only one
 *  platform keeps working on the other (it becomes an entry for that platform), and an import with no `platform`
 *  replaces the command's entries for either platform: no command is left with two entries for one platform. */
export function mergeImport(current: unknown[], usable: unknown[]): unknown[] {
    const covered = new Map<string, Set<string>>();
    for (const u of usable) {
        const set = covered.get(commandOf(u)) ?? new Set<string>();
        platformsOf(u).forEach((p) => set.add(p));
        covered.set(commandOf(u), set);
    }
    const keep: unknown[] = [];
    for (const c of current) {
        const cover = entryIssue(c) == null ? covered.get(commandOf(c)) : undefined;
        if (!cover) {
            keep.push(c);
            continue;
        }
        const left = platformsOf(c).filter((p) => !cover.has(p));
        if (left.length === platformsOf(c).length) keep.push(c);
        else if (left.length === 1) keep.push({ ...(c as Entry), platform: left[0] });
    }
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
