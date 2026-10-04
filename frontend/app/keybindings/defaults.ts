// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The default shortcut table: the one place a global shortcut is defined. The
// dispatcher, the terminal, the help pane, the menus and the docs all read it
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §6, §12).

export type KeyCategory = "Tabs & windows" | "Panes" | "Find & zoom" | "Terminal" | "General";

export interface KeyBindingRow {
    command: string;
    label: string;
    category: KeyCategory;
    /** Keys on macOS (keys.ts syntax). A chord is two keys separated by a space. */
    mac?: string[];
    /** Keys on Windows/Linux. Window, tab and pane actions use Ctrl+Shift so
     *  Alt+letter stays free for the shell (report §11.2). */
    other?: string[];
    /** `KeyContext` flags (registry.ts) joined with `&&`, each optionally `!`. */
    when?: string;
    /** Also fires while a terminal has focus; every other binding leaves the
     *  key to the shell there (report §11.3). */
    skipShell?: boolean;
    /** Rows sharing a group show as one help line ("Go to tab 1–8"). */
    helpGroup?: string;
    /** Dev builds only: left out of the help pane. Its handler is registered
     *  by the dev panel itself, so in a release build the key does nothing. */
    devOnly?: boolean;
}

export const DEFAULT_KEYBINDINGS: KeyBindingRow[] = [
    // ── General ──
    { command: "view:command-palette", label: "Command palette", category: "General", mac: ["meta+shift+p", "meta+p"], other: ["ctrl+shift+p"], skipShell: true },
    // Ctrl+P is the shell's previous-history key, so not in a terminal.
    { command: "view:command-palette", label: "Command palette", category: "General", other: ["ctrl+p"] },
    { command: "app:settings", label: "Settings", category: "General", mac: ["meta+,"], other: ["ctrl+,"], skipShell: true },
    { command: "help:shortcuts", label: "Keyboard shortcuts", category: "General", mac: ["meta+/", "F1"], other: ["ctrl+/", "F1"], skipShell: true },

    // ── Tabs & windows ──
    { command: "window:new", label: "New window", category: "Tabs & windows", mac: ["meta+shift+n"], other: ["ctrl+shift+n"], skipShell: true },
    { command: "tab:new", label: "New tab", category: "Tabs & windows", mac: ["meta+t"], other: ["ctrl+shift+t"], skipShell: true },
    { command: "tab:close", label: "Close tab", category: "Tabs & windows", mac: ["meta+shift+w"], other: ["ctrl+shift+alt+w", "ctrl+F4"], skipShell: true },
    { command: "tab:next", label: "Next tab", category: "Tabs & windows", mac: ["meta+]", "meta+shift+]", "ctrl+Tab"], other: ["ctrl+shift+]", "ctrl+Tab"], skipShell: true },
    { command: "tab:prev", label: "Previous tab", category: "Tabs & windows", mac: ["meta+[", "meta+shift+[", "ctrl+shift+Tab"], other: ["ctrl+shift+[", "ctrl+shift+Tab"], skipShell: true },
    ...[1, 2, 3, 4, 5, 6, 7, 8].map(
        (n): KeyBindingRow => ({ command: `tab:goto:${n}`, label: `Go to tab ${n}`, helpGroup: "Go to tab 1–8", category: "Tabs & windows", mac: [`meta+${n}`], other: [`ctrl+${n}`], skipShell: true })
    ),
    { command: "tab:goto:last", label: "Go to last tab", category: "Tabs & windows", mac: ["meta+9"], other: ["ctrl+9"], skipShell: true },
    { command: "tab:moveLeft", label: "Move tab left", category: "Tabs & windows", mac: ["meta+shift+PageUp"], other: ["ctrl+alt+shift+PageUp"], skipShell: true },
    { command: "tab:moveRight", label: "Move tab right", category: "Tabs & windows", mac: ["meta+shift+PageDown"], other: ["ctrl+alt+shift+PageDown"], skipShell: true },
    // F2 renames whatever has focus (a file in the Files pane or editor tree
    // handles it first), so the tab only takes it when nothing else does.
    { command: "tab:rename", label: "Rename tab", category: "Tabs & windows", mac: ["F2"], other: ["F2"], when: "!textInputFocus" },

    // ── Panes ──
    { command: "pane:new", label: "New pane", category: "Panes", mac: ["meta+n"], other: ["ctrl+shift+code:Backquote"], skipShell: true },
    { command: "open:agent", label: "New agent pane", category: "Panes", mac: ["meta+shift+a"], other: ["ctrl+shift+a"], skipShell: true },
    { command: "split:right", label: "Split right", category: "Panes", mac: ["meta+d"], other: ["ctrl+shift+d"], skipShell: true },
    { command: "split:down", label: "Split below", category: "Panes", mac: ["meta+shift+d"], other: ["ctrl+shift+alt+d"], skipShell: true },
    { command: "split:up", label: "Split above", helpGroup: "Split in a direction", category: "Panes", mac: ["ctrl+shift+s ArrowUp"], other: ["ctrl+shift+s ArrowUp"], skipShell: true },
    { command: "split:down", label: "Split below", helpGroup: "Split in a direction", category: "Panes", mac: ["ctrl+shift+s ArrowDown"], other: ["ctrl+shift+s ArrowDown"], skipShell: true },
    { command: "split:left", label: "Split left", helpGroup: "Split in a direction", category: "Panes", mac: ["ctrl+shift+s ArrowLeft"], other: ["ctrl+shift+s ArrowLeft"], skipShell: true },
    { command: "split:right", label: "Split right", helpGroup: "Split in a direction", category: "Panes", mac: ["ctrl+shift+s ArrowRight"], other: ["ctrl+shift+s ArrowRight"], skipShell: true },
    { command: "pane:close", label: "Close pane", category: "Panes", mac: ["meta+w"], other: ["ctrl+shift+w"], skipShell: true },
    { command: "pane:magnify", label: "Maximize pane", category: "Panes", mac: ["meta+m"], other: ["ctrl+shift+m"], skipShell: true },
    { command: "pane:focus:up", label: "Focus pane above", helpGroup: "Focus pane by direction", category: "Panes", mac: ["ctrl+shift+ArrowUp"], other: ["ctrl+shift+ArrowUp"], when: "!textInputFocus", skipShell: true },
    { command: "pane:focus:down", label: "Focus pane below", helpGroup: "Focus pane by direction", category: "Panes", mac: ["ctrl+shift+ArrowDown"], other: ["ctrl+shift+ArrowDown"], when: "!textInputFocus", skipShell: true },
    { command: "pane:focus:left", label: "Focus pane left", helpGroup: "Focus pane by direction", category: "Panes", mac: ["ctrl+shift+ArrowLeft"], other: ["ctrl+shift+ArrowLeft"], when: "!textInputFocus", skipShell: true },
    { command: "pane:focus:right", label: "Focus pane right", helpGroup: "Focus pane by direction", category: "Panes", mac: ["ctrl+shift+ArrowRight"], other: ["ctrl+shift+ArrowRight"], when: "!textInputFocus", skipShell: true },
    { command: "pane:focus:next", label: "Next pane", category: "Panes", mac: ["F6"], other: ["F6"], skipShell: true },
    { command: "pane:focus:prev", label: "Previous pane", category: "Panes", mac: ["shift+F6"], other: ["shift+F6"], skipShell: true },
    ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map(
        (n): KeyBindingRow => ({
            command: `pane:focus:${n}`,
            label: `Focus pane ${n}`,
            helpGroup: "Focus pane 1–9",
            category: "Panes",
            mac: [`ctrl+shift+code:Digit${n}`, `ctrl+shift+code:Numpad${n}`],
            other: [`ctrl+shift+code:Digit${n}`, `ctrl+shift+code:Numpad${n}`],
            skipShell: true,
        })
    ),
    { command: "pane:swap:up", label: "Swap with pane above", helpGroup: "Swap pane with neighbour", category: "Panes", mac: ["ctrl+alt+shift+ArrowUp"], other: ["ctrl+alt+shift+ArrowUp"], when: "!textInputFocus", skipShell: true },
    { command: "pane:swap:down", label: "Swap with pane below", helpGroup: "Swap pane with neighbour", category: "Panes", mac: ["ctrl+alt+shift+ArrowDown"], other: ["ctrl+alt+shift+ArrowDown"], when: "!textInputFocus", skipShell: true },
    { command: "pane:swap:left", label: "Swap with pane left", helpGroup: "Swap pane with neighbour", category: "Panes", mac: ["ctrl+alt+shift+ArrowLeft"], other: ["ctrl+alt+shift+ArrowLeft"], when: "!textInputFocus", skipShell: true },
    { command: "pane:swap:right", label: "Swap with pane right", helpGroup: "Swap pane with neighbour", category: "Panes", mac: ["ctrl+alt+shift+ArrowRight"], other: ["ctrl+alt+shift+ArrowRight"], when: "!textInputFocus", skipShell: true },
    { command: "pane:resize:up", label: "Move pane border up", helpGroup: "Resize pane", category: "Panes", mac: ["ctrl+alt+meta+ArrowUp"], other: ["alt+shift+ArrowUp"], when: "!textInputFocus", skipShell: true },
    { command: "pane:resize:down", label: "Move pane border down", helpGroup: "Resize pane", category: "Panes", mac: ["ctrl+alt+meta+ArrowDown"], other: ["alt+shift+ArrowDown"], when: "!textInputFocus", skipShell: true },
    { command: "pane:resize:left", label: "Move pane border left", helpGroup: "Resize pane", category: "Panes", mac: ["ctrl+alt+meta+ArrowLeft"], other: ["alt+shift+ArrowLeft"], when: "!textInputFocus", skipShell: true },
    { command: "pane:resize:right", label: "Move pane border right", helpGroup: "Resize pane", category: "Panes", mac: ["ctrl+alt+meta+ArrowRight"], other: ["alt+shift+ArrowRight"], when: "!textInputFocus", skipShell: true },
    { command: "pane:refocus", label: "Refocus pane", category: "Panes", mac: ["meta+i"] },
    { command: "agent:focusComposer", label: "Focus the message box", category: "Panes", mac: ["meta+l"], other: ["ctrl+l"], when: "viewType == agent" },
    { command: "pane:replaceWithLauncher", label: "Replace pane with launcher", category: "Panes", mac: ["ctrl+shift+k"], other: ["ctrl+shift+k"], when: "!textInputFocus", skipShell: true },
    { command: "pane:changeConnection", label: "Change connection", category: "Panes", mac: ["meta+shift+g"], other: ["ctrl+shift+g"], skipShell: true },
    { command: "term:multiInput", label: "Type into all terminals", category: "Terminal", mac: ["meta+shift+m"], other: ["ctrl+shift+alt+m"], skipShell: true },
    { command: "pane:voice", label: "Voice input", category: "General", mac: ["ctrl+shift+v"], other: ["ctrl+shift+v"], when: "!terminalFocus" },

    // ── Find & zoom ──
    { command: "pane:find", label: "Find in pane", category: "Find & zoom", mac: ["meta+f"], skipShell: true },
    // Ctrl+F is the shell's forward-char key: in a terminal, find is Ctrl+Shift+F.
    { command: "pane:find", label: "Find in pane", category: "Find & zoom", other: ["ctrl+f"] },
    { command: "pane:find", label: "Find in pane", category: "Find & zoom", other: ["ctrl+shift+f"], when: "terminalFocus", skipShell: true },
    { command: "app:escape", label: "Close dialog or find bar", category: "General", mac: ["Escape"], other: ["Escape"] },
    { command: "view:zoom:in", label: "Zoom in", category: "Find & zoom", mac: ["meta+=", "meta+shift+=", "meta+code:NumpadAdd"], other: ["ctrl+=", "ctrl+shift+=", "ctrl+code:NumpadAdd"], skipShell: true },
    { command: "view:zoom:out", label: "Zoom out", category: "Find & zoom", mac: ["meta+-", "meta+code:NumpadSubtract"], other: ["ctrl+-", "ctrl+code:NumpadSubtract"], skipShell: true },
    { command: "view:zoom:reset", label: "Reset zoom", category: "Find & zoom", mac: ["meta+0", "meta+code:Numpad0"], other: ["ctrl+0", "ctrl+code:Numpad0"], skipShell: true },
    { command: "view:zoom:resetAll", label: "Reset zoom on all panes", category: "Find & zoom", mac: ["meta+shift+0"], other: ["ctrl+shift+0"], skipShell: true },

    // ── Dev builds ──
    { command: "dev:perfHud", label: "Performance HUD", category: "General", mac: ["meta+alt+shift+p"], other: ["ctrl+alt+shift+p"], skipShell: true, devOnly: true },
    { command: "dev:diagnostics", label: "Diagnostics panel", category: "General", mac: ["meta+alt+shift+F12"], other: ["ctrl+alt+shift+F12"], skipShell: true, devOnly: true },

    // ── Terminal (run by the terminal itself) ──
    // macOS: ⌘C / ⌘V already copy and paste natively in the terminal.
    { command: "term:copy", label: "Copy", category: "Terminal", other: ["ctrl+shift+c"], when: "terminalFocus", skipShell: true },
    { command: "term:paste", label: "Paste", category: "Terminal", other: ["ctrl+shift+v"], when: "terminalFocus", skipShell: true },
    { command: "term:clear", label: "Clear", category: "Terminal", mac: ["meta+k"], other: ["ctrl+shift+l"], when: "terminalFocus", skipShell: true },
];
