// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The help pane's shortcut list, generated from the default table so it can't
// drift from what the keys do.

import { DEFAULT_KEYBINDINGS, type KeyBindingRow, type KeyCategory } from "./defaults";
import { formatKey, type KeyPlatform } from "./keys";

export interface HelpEntry {
    label: string;
    keys: string[];
}

export interface HelpSection {
    category: KeyCategory;
    entries: HelpEntry[];
}

const CATEGORY_ORDER: KeyCategory[] = ["General", "Tabs & windows", "Panes", "Find & zoom", "Terminal"];

/** "Ctrl+1"…"Ctrl+8" → "Ctrl+1–8"; "Ctrl+Shift+↑"… → "Ctrl+Shift+↑/↓/←/→". */
export function compactKeys(labels: string[]): string {
    if (labels.length < 2) return labels[0] ?? "";
    let prefix = labels[0];
    for (const l of labels) {
        while (!l.startsWith(prefix)) prefix = prefix.slice(0, -1);
    }
    const rest = labels.map((l) => l.slice(prefix.length));
    if (rest.some((r) => r === "")) return labels.join(", ");
    const nums = rest.map(Number);
    if (rest.every((r) => /^\d$/.test(r)) && nums.every((n, i) => i === 0 || n === nums[i - 1] + 1)) {
        return `${prefix}${rest[0]}–${rest[rest.length - 1]}`;
    }
    return `${prefix}${rest.join("/")}`;
}

function rowKeys(row: KeyBindingRow, platform: KeyPlatform): string[] {
    return (platform === "mac" ? row.mac : row.other) ?? [];
}

export function helpSections(platform: KeyPlatform): HelpSection[] {
    const entries = new Map<string, { category: KeyCategory; entry: HelpEntry; groupKeys: string[] }>();
    for (const row of DEFAULT_KEYBINDINGS) {
        const keys = rowKeys(row, platform);
        if (keys.length === 0) continue;
        const id = row.helpGroup ?? row.command;
        let e = entries.get(id);
        if (!e) {
            e = { category: row.category, entry: { label: row.helpGroup ?? row.label, keys: [] }, groupKeys: [] };
            entries.set(id, e);
        }
        if (row.helpGroup) {
            e.groupKeys.push(formatKey(keys[0], platform));
        } else {
            const suffix = row.when === "terminalFocus" && row.category !== "Terminal" ? " (in a terminal)" : "";
            for (const k of keys.slice(0, 2)) e.entry.keys.push(formatKey(k, platform) + suffix);
        }
    }
    const sections = new Map<KeyCategory, HelpEntry[]>();
    for (const { category, entry, groupKeys } of entries.values()) {
        if (groupKeys.length > 0) entry.keys = [compactKeys(groupKeys)];
        if (!sections.has(category)) sections.set(category, []);
        sections.get(category)!.push(entry);
    }
    return CATEGORY_ORDER.filter((c) => sections.has(c)).map((category) => ({ category, entries: sections.get(category)! }));
}
