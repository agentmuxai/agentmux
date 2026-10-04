// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// One key syntax for every binding, its matcher and its label
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §6.2, §11.4).

export type KeyPlatform = "mac" | "other";

export interface KeySpec {
    ctrl: boolean;
    shift: boolean;
    alt: boolean;
    meta: boolean;
    /** A lower-case letter a–z, matched by `key` then `code`. */
    letter?: string;
    /** A physical key, matched by `event.code`. */
    code?: string;
    /** A named key (`Enter`, `ArrowUp`, `F6`…), matched by `event.key`. */
    named?: string;
}

export interface KeyEventLike {
    key: string;
    code: string;
    ctrlKey: boolean;
    shiftKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    getModifierState?: (key: string) => boolean;
}

const PUNCT_CODE: Record<string, string> = {
    "[": "BracketLeft",
    "]": "BracketRight",
    "=": "Equal",
    "-": "Minus",
    ",": "Comma",
    ".": "Period",
    "/": "Slash",
    "\\": "Backslash",
    ";": "Semicolon",
    "'": "Quote",
    "`": "Backquote",
};

const CODE_LABEL: Record<string, string> = {
    ...Object.fromEntries(Object.entries(PUNCT_CODE).map(([ch, code]) => [code, ch])),
    NumpadAdd: "Num+",
    NumpadSubtract: "Num-",
};

const NAMED_LABEL: Record<string, string> = {
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    Escape: "Esc",
    PageUp: "PgUp",
    PageDown: "PgDn",
    " ": "Space",
};

/**
 * Parses `"ctrl+shift+t"`, `"meta+["`, `"mod+f"`, `"shift+F6"`, `"ctrl+code:Digit1"`.
 * `mod` is ⌘ on macOS and Ctrl elsewhere. Letters match the typed character,
 * falling back to the physical key; digits and punctuation match `event.code`,
 * so they work on AZERTY and German layouts. `code:X` names a physical key.
 */
export function parseKey(spec: string, platform: KeyPlatform): KeySpec {
    const parts = spec.split("+");
    // "ctrl++" can't be written; the key itself is never "+" (use "shift+=").
    const keyTok = parts.pop() ?? "";
    const out: KeySpec = { ctrl: false, shift: false, alt: false, meta: false };
    for (const raw of parts) {
        const m = raw.toLowerCase();
        if (m === "mod") {
            if (platform === "mac") out.meta = true;
            else out.ctrl = true;
        } else if (m === "ctrl" || m === "shift" || m === "alt" || m === "meta") {
            out[m] = true;
        } else {
            throw new Error(`keybinding "${spec}": unknown modifier "${raw}"`);
        }
    }
    if (keyTok.startsWith("code:")) {
        out.code = keyTok.slice(5);
    } else if (/^[a-zA-Z]$/.test(keyTok)) {
        out.letter = keyTok.toLowerCase();
    } else if (/^[0-9]$/.test(keyTok)) {
        out.code = `Digit${keyTok}`;
    } else if (PUNCT_CODE[keyTok]) {
        out.code = PUNCT_CODE[keyTok];
    } else if (keyTok.length > 1) {
        out.named = keyTok;
    } else {
        throw new Error(`keybinding "${spec}": unsupported key "${keyTok}"`);
    }
    return out;
}

export function matchKey(e: KeyEventLike, k: KeySpec): boolean {
    if (e.ctrlKey !== k.ctrl || e.shiftKey !== k.shift || e.altKey !== k.alt || e.metaKey !== k.meta) {
        return false;
    }
    if (k.named) return e.key === k.named;
    if (k.code) return e.code === k.code;
    if (k.letter) {
        const typed = e.key.length === 1 ? e.key.toLowerCase() : "";
        if (typed === k.letter) return true;
        // AltGr is Ctrl+Alt on Windows. When it types a character on this
        // layout, the user meant the character, not a Ctrl+Alt shortcut.
        if (k.ctrl && k.alt && e.getModifierState?.("AltGraph") && typed !== "" && !/^[a-z]$/.test(typed)) {
            return false;
        }
        // The typed character isn't a Latin letter (another script, or a
        // modifier changed it): fall back to the physical key.
        return !/^[a-z]$/.test(typed) && e.code === `Key${k.letter.toUpperCase()}`;
    }
    return false;
}

export function sameKey(a: KeySpec, b: KeySpec): boolean {
    return (
        a.ctrl === b.ctrl &&
        a.shift === b.shift &&
        a.alt === b.alt &&
        a.meta === b.meta &&
        a.letter === b.letter &&
        a.code === b.code &&
        a.named === b.named
    );
}

function keyLabel(k: KeySpec): string {
    if (k.letter) return k.letter.toUpperCase();
    if (k.code) return CODE_LABEL[k.code] ?? k.code.replace(/^(Digit|Numpad)/, (m) => (m === "Numpad" ? "Num" : ""));
    return NAMED_LABEL[k.named ?? ""] ?? k.named ?? "";
}

/** "Ctrl+Shift+T" elsewhere, "⇧⌘T" on macOS (Apple's modifier order). */
export function formatKey(spec: string, platform: KeyPlatform): string {
    return spec
        .split(" ")
        .map((step) => {
            const k = parseKey(step, platform);
            if (platform === "mac") {
                return `${k.ctrl ? "⌃" : ""}${k.alt ? "⌥" : ""}${k.shift ? "⇧" : ""}${k.meta ? "⌘" : ""}${keyLabel(k)}`;
            }
            const mods = [k.ctrl && "Ctrl", k.alt && "Alt", k.shift && "Shift", k.meta && "Win"].filter(Boolean);
            return [...mods, keyLabel(k)].join("+");
        })
        .join(" then ");
}
