// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The app shortcuts the host forwards out of a focused browser pane
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §6.5).
// A browser pane is a native CEF child window, so its keys never reach the
// app's DOM listeners; crates/cef reads host-keys.json, which this generates.

import { DEFAULT_KEYBINDINGS, type KeyBindingRow } from "./defaults";
import { parseKey, type KeyPlatform, type KeySpec } from "./keys";

export interface HostKey {
    command: string;
    ctrl: boolean;
    shift: boolean;
    alt: boolean;
    meta: boolean;
    /** Windows virtual-key code; CEF reports these on every platform. */
    vk: number;
}

const NAMED_VK: Record<string, number> = {
    Tab: 0x09,
    Enter: 0x0d,
    Escape: 0x1b,
    PageUp: 0x21,
    PageDown: 0x22,
    ArrowLeft: 0x25,
    ArrowUp: 0x26,
    ArrowRight: 0x27,
    ArrowDown: 0x28,
    Delete: 0x2e,
    ...Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`F${i + 1}`, 0x70 + i])),
};

const CODE_VK: Record<string, number> = {
    ...Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`Digit${i}`, 0x30 + i])),
    ...Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`Numpad${i}`, 0x60 + i])),
    NumpadAdd: 0x6b,
    NumpadSubtract: 0x6d,
    Semicolon: 0xba,
    Equal: 0xbb,
    Comma: 0xbc,
    Minus: 0xbd,
    Period: 0xbe,
    Slash: 0xbf,
    Backquote: 0xc0,
    BracketLeft: 0xdb,
    Backslash: 0xdc,
    BracketRight: 0xdd,
    Quote: 0xde,
};

function vkOf(k: KeySpec): number | null {
    if (k.letter) return k.letter.toUpperCase().charCodeAt(0);
    if (k.code) return CODE_VK[k.code] ?? null;
    return NAMED_VK[k.named ?? ""] ?? null;
}

/**
 * Rows worth forwarding: global (not a pane's), allowed in a terminal too
 * (`skipShell`, so they don't take a key a page commonly needs), single-key,
 * not find or zoom (a page has its own), and not scoped to typing or to a
 * terminal: the host can't tell whether the page has a text field focused.
 */
function forwards(row: KeyBindingRow): boolean {
    if (row.pane || !row.skipShell) return false;
    if (row.category === "Find & zoom" || row.category === "Terminal") return false;
    const terms = (row.when ?? "").split("&&").map((t) => t.trim()).filter(Boolean);
    // A browser pane is neither a document-tab host nor any excluded view.
    return terms.every((t) => t === "!docTabsHost" || /^viewType\s*!=/.test(t));
}

export function hostKeys(platform: KeyPlatform): HostKey[] {
    const out: HostKey[] = [];
    for (const row of DEFAULT_KEYBINDINGS) {
        if (!forwards(row)) continue;
        for (const source of (platform === "mac" ? row.mac : row.other) ?? []) {
            if (source.includes(" ")) continue; // chords
            const k = parseKey(source, platform);
            const vk = vkOf(k);
            if (vk == null) continue;
            out.push({ command: row.command, ctrl: k.ctrl, shift: k.shift, alt: k.alt, meta: k.meta, vk });
        }
    }
    return out;
}
