// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a key press becomes when a person records a shortcut: the string the `keybindings` setting uses
// (`parseKey` reads it back). Pure, so the Settings recorder and its tests share one rule
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4.5).

import { tokenForCode, type KeyEventLike } from "./keys";

export interface CaptureEvent extends KeyEventLike {
    isComposing?: boolean;
    repeat?: boolean;
}

/** The key as a setting string, or why the press is not a shortcut. */
export type Captured = { key: string } | { ignored: "modifier" | "composing" | "repeat" | "unknown" };

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph", "OS", "Hyper", "Super", "CapsLock", "NumLock", "ScrollLock", "Fn", "FnLock"]);
/** Keys whose `key` says nothing about which key it was: the physical key is used instead. */
const NO_NAME = new Set(["Dead", "Unidentified", "Process"]);

/** The key itself, without modifiers: a letter by what it types (by its physical key when that is not a Latin
 *  letter, as on macOS with Option), a digit or punctuation by its physical key (so AZERTY and German layouts keep
 *  working), anything else by name or, failing that, by physical key. */
function tokenOf(e: CaptureEvent): string | null {
    if (/^[a-zA-Z]$/.test(e.key)) return e.key.toLowerCase();
    const letter = e.code.match(/^Key([A-Z])$/);
    if (letter) return letter[1].toLowerCase();
    const physical = tokenForCode(e.code);
    if (physical) return physical;
    if (e.key === " ") return "code:Space";
    if (e.key.length > 1 && !NO_NAME.has(e.key)) return e.key;
    return e.code ? `code:${e.code}` : null;
}

/** A recorded press: modifiers in a fixed order (Ctrl, Alt, Shift, Meta), then the key. A modifier alone, a press
 *  while an input method is composing and a key held down are not shortcuts. */
export function captureKey(e: CaptureEvent): Captured {
    if (e.isComposing || e.key === "Process") return { ignored: "composing" };
    if (e.repeat) return { ignored: "repeat" };
    if (MODIFIER_KEYS.has(e.key)) return { ignored: "modifier" };
    const token = tokenOf(e);
    if (!token) return { ignored: "unknown" };
    const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "meta"].filter(Boolean);
    return { key: [...mods, token].join("+") };
}
