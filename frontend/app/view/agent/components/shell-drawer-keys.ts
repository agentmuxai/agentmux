// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Clipboard keys for the agent pane's Shell drawer terminal — Ctrl+Shift+V
 * paste and Ctrl+Shift+C copy, the same chords a terminal pane handles in
 * `TermViewModel.handleTerminalKeydown`.
 *
 * Found in the running app: with no key handler on the drawer's xterm,
 * Ctrl+Shift+V fell through to the app-level global hotkey (keymodel.ts) that
 * starts voice dictation into the agent pane — so "paste" turned the
 * microphone on — and Ctrl+Shift+C did nothing. Plain Ctrl+V already worked
 * (native paste event → xterm) and is left alone.
 */

import * as keyutil from "@/util/keyutil";
import { pasteClipboardIntoShell, type ShellDrawerMenuDeps } from "./shell-drawer-menu";

/**
 * xterm `attachCustomKeyEventHandler` contract: return `true` to let xterm
 * process the event normally, `false` when it was handled here.
 */
export function handleShellDrawerKeydown(event: KeyboardEvent, deps: ShellDrawerMenuDeps): boolean {
    const muxEvent = keyutil.adaptFromReactOrNativeKeyEvent(event);
    // xterm calls the handler for keydown, keypress and keyup; act once.
    if (muxEvent.type != "keydown") return true;

    if (keyutil.checkKeyPressed(muxEvent, "Ctrl:Shift:v")) {
        // preventDefault + stopPropagation: the global Ctrl+Shift+V (voice)
        // hotkey listens above us and must not also fire.
        event.preventDefault();
        event.stopPropagation();
        void pasteClipboardIntoShell(deps);
        return false;
    }
    if (keyutil.checkKeyPressed(muxEvent, "Ctrl:Shift:c")) {
        event.preventDefault();
        event.stopPropagation();
        // Swallowed even with no selection, as in terminal panes: xterm would
        // otherwise send the chord to the shell as ^C.
        const selection = deps.getTerminal()?.getSelection() ?? "";
        if (selection) deps.writeClipboard(selection).catch((e) => console.error("[shell-drawer] copy failed:", e));
        return false;
    }
    return true;
}
