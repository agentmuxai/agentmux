// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Keys a focused terminal gives the shell before any app shortcut sees them:
// readline's Alt+letter Meta keys on Windows/Linux (where `Cmd:` is Alt, so
// Alt+W used to close the pane), Ctrl+P (history), Ctrl+[ (vim's Escape) and
// Ctrl+]. See docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §3.2.

export interface ShellKeyEvent {
    code: string;
    ctrlKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
}

export function terminalKeyGoesToShell(e: ShellKeyEvent, isMac: boolean): boolean {
    // AltGr is Ctrl+Alt on Windows, so requiring !ctrlKey also leaves
    // AltGr-typed characters alone.
    if (!isMac && e.altKey && !e.ctrlKey && !e.metaKey && /^Key[A-Z]$/.test(e.code)) {
        return true;
    }
    if (e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
        return e.code === "KeyP" || e.code === "BracketLeft" || e.code === "BracketRight";
    }
    return false;
}
