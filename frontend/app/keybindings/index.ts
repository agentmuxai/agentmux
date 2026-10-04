// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shortcut labels for the current platform. Every place the UI shows a key
// uses these, so a hint can't disagree with the binding.

import { isMacOS } from "@/util/platformutil";
import type { KeyPane } from "./defaults";
import { helpSections, type HelpSection } from "./help";
import { formatKey, parseKey, type KeyEventLike, type KeyPlatform } from "./keys";
import { formatCommand, keysFor, matchPaneKey } from "./registry";

export function keyPlatform(): KeyPlatform {
    return isMacOS() ? "mac" : "other";
}

/** The shortcut for a command ("Ctrl+Shift+T", "⌘T"), or "" if it has none. */
export function shortcutFor(command: string): string {
    return formatCommand(command, keyPlatform());
}

/** A component-local key in this platform's notation: `keyLabel("mod+s")`. */
export function keyLabel(spec: string): string {
    return formatKey(spec, keyPlatform());
}

export function shortcutHelp(): HelpSection[] {
    return helpSections(keyPlatform());
}

/** A DOM key event as the matcher reads it. Field by field: a KeyboardEvent's
 *  fields are prototype getters, which a spread would drop. */
export function keyEventLike(e: KeyboardEvent): KeyEventLike {
    return {
        key: e.key,
        code: e.code,
        ctrlKey: e.ctrlKey,
        shiftKey: e.shiftKey,
        altKey: e.altKey,
        metaKey: e.metaKey,
        getModifierState: e.getModifierState?.bind(e),
    };
}

/** The command a pane's own handler should run for `e` (its `pane` rows). */
export function paneCommandFor(e: KeyboardEvent, pane: KeyPane): string | null {
    return matchPaneKey(keyEventLike(e), pane, keyPlatform());
}

/** A command's keys in CodeMirror's syntax ("Ctrl-Shift-s", "Meta-s"), so an
 *  editor keymap comes from the table. Single-key bindings only. */
export function codeMirrorKeys(command: string): string[] {
    const platform = keyPlatform();
    return keysFor(command, platform)
        .filter((k) => !k.includes(" "))
        .map((k) => {
            const spec = parseKey(k, platform);
            const mods = [spec.ctrl && "Ctrl", spec.alt && "Alt", spec.shift && "Shift", spec.meta && "Meta"].filter(Boolean);
            return [...mods, spec.letter ?? spec.named ?? spec.code].join("-");
        });
}
