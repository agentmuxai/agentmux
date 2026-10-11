// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shortcut labels for the current platform. Every place the UI shows a key
// uses these, so a hint can't disagree with the binding.

import { isLinux, isMacOS } from "@/util/platformutil";
import { noteResolved } from "./app-api";
export { registerPaneCommandRunner } from "./app-api";
import type { KeyPane } from "./defaults";
import { helpSections, type HelpSection } from "./help";
import { formatKey, parseKey, type KeyEventLike, type KeyPlatform } from "./keys";
import { formatCommand, keybindingsVersion, keysFor, matchPaneKey, resolveKey, type KeyContext } from "./registry";

let keyContext: () => KeyContext = () => ({ textInputFocus: false, terminalFocus: false, viewType: "", docTabsHost: false });

/** Where focus is, for `isGlobalKey`: the dispatcher supplies it at load
 *  (keymodel-dispatch's currentKeyContext), which avoids an import cycle. */
export function setKeyContextProvider(provider: () => KeyContext): void {
    keyContext = provider;
}

/**
 * Whether the shortcut table gives this key a global command where focus is
 * now. A pane's own key handler leaves such a key to the dispatcher, even an
 * arrow or Page key it would otherwise take (⌃⇧↑ in the Files list).
 */
export function isGlobalKey(e: KeyboardEvent): boolean {
    return resolveKey(keyEventLike(e), keyContext(), keyPlatform()) != null;
}

export function keyPlatform(): KeyPlatform {
    return isMacOS() ? "mac" : isLinux() ? "linux" : "other";
}

/** The shortcut for a command ("Ctrl+Shift+T", "⌘T"), or "" if it has none. */
export function shortcutFor(command: string): string {
    return formatCommand(command, keyPlatform());
}

/** A component-local key in this platform's notation: `keyLabel("mod+s")`. */
export function keyLabel(spec: string): string {
    return formatKey(spec, keyPlatform());
}

/** Reactive: an open help pane re-renders when your keybindings change. */
export function shortcutHelp(): HelpSection[] {
    keybindingsVersion();
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
    const command = matchPaneKey(keyEventLike(e), pane, keyPlatform());
    if (command) noteResolved(command, pane);
    return command;
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
