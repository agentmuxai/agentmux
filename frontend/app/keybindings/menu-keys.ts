// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The keys the macOS menu bar shows next to its items. The menu is native and built by the host, which can't read
// the app's shortcut table, so the app sends it the table's answer for each command: the key a person presses for
// it, as the table resolves it now (their own binding first, else the default). The host puts that key on the
// item with the same command id (crates/cef/src/macos_menu.rs), so the menu shows what the palette and the
// Keyboard settings show.

import { tokenForCode, parseKey, type KeyPlatform } from "./keys";
import type { KeyBindingRow } from "./defaults";

/** One command's key: the character AppKit takes as a key equivalent, and the modifiers held with it. */
export interface MenuKey {
    command: string;
    /** One lower-case letter, digit or punctuation character; Shift is `shift`, not an upper-case letter. */
    key: string;
    ctrl: boolean;
    shift: boolean;
    alt: boolean;
    meta: boolean;
}

/** The character of a key a menu item can carry: a letter, a digit or a punctuation key, not a named key (Tab,
 *  arrows, F-keys) or the number pad. */
function keyChar(source: string, platform: KeyPlatform): Omit<MenuKey, "command"> | null {
    const k = parseKey(source, platform);
    const key = k.letter ?? (k.code ? tokenForCode(k.code) : null);
    // A key with no modifier would take every press of that character from the page.
    if (!key || !(k.ctrl || k.alt || k.meta)) return null;
    return { key, ctrl: k.ctrl, shift: k.shift, alt: k.alt, meta: k.meta };
}

/**
 * The key for each command that has one a menu item can show: the first binding, the person's own before the
 * defaults, that is one key (not a chord), holds everywhere (a row with a `when` or for one pane would fire from the
 * menu where the page's handler would not) and is a character key.
 */
export function menuKeysOf(rows: { row: KeyBindingRow; source: string }[], platform: KeyPlatform): MenuKey[] {
    const out: MenuKey[] = [];
    const done = new Set<string>();
    for (const { row, source } of rows) {
        if (done.has(row.command) || row.pane || row.when || source.includes(" ")) continue;
        const key = keyChar(source, platform);
        if (!key) continue;
        done.add(row.command);
        out.push({ command: row.command, ...key });
    }
    return out;
}
