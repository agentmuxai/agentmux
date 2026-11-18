// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Palette entries for the commands the shortcut table binds to a key, so a person who cannot remember a
 * chord can still find the command by name. Each runs the code its key runs: a global command through the
 * dispatcher's handler, a pane's own command through the focused pane's runner (`window.__agentmux_shortcuts`).
 * The ids are the table's command ids, so the palette shows the key beside the name (`shortcutFor`).
 *
 * `KEY_COMMANDS` is the hand-written list of the global ones (a readable name, an icon, search words); the
 * pane-scoped rows (Files, Editor, Documents, Browser, Terminal) are derived from the table and show only
 * while a pane of that kind has focus. `KEY_COMMANDS_WITHOUT_ENTRY` lists, with its reason, every table
 * command that has no entry; `command-registry-coverage.test.ts` fails on a table row in neither place.
 */

import { DEFAULT_KEYBINDINGS, type KeyBindingRow, type KeyPane } from "@/app/keybindings/defaults";
import { DOC_TAB_HOSTS } from "@/app/keybindings/registry";
import { supportsBackdrop } from "@/app/backdrop/meta";
import { openBackdropPicker } from "@/app/backdrop/picker-state";
import { getBlockComponentModel, getFocusedBlockId } from "@/app/store/block-component-registry";
import type { CommandEntry } from "@/app/store/command-registry";
import { devModeOn } from "@/app/store/dev-mode";
import { openMemory } from "@/app/store/section-panes";

/** The view type of the focused pane, if one has focus. */
function focusedView(): string | undefined {
    try {
        const blockId = getFocusedBlockId();
        return blockId ? getBlockComponentModel(blockId)?.viewModel?.viewType : undefined;
    } catch {
        return undefined;
    }
}

interface KeyCommandSpec {
    id: string;
    label: string;
    category: string;
    icon?: string;
    keywords?: string;
    /** Only while a pane of this view type has focus. */
    view?: string;
    /** Only in dev mode. */
    dev?: boolean;
}

export const KEY_COMMANDS: readonly KeyCommandSpec[] = [
    { id: "help:shortcuts", label: "Keyboard shortcuts", category: "Help", icon: "keyboard", keywords: "keys hotkeys chords list sheet help" },
    { id: "pane:new", label: "New Pane", category: "Pane", icon: "plus", keywords: "add open pane launcher" },
    { id: "tab:rename", label: "Rename Tab", category: "Tab", icon: "pen", keywords: "name title tab" },
    { id: "tab:moveLeft", label: "Move Tab Left", category: "Tab", icon: "arrow-left", keywords: "reorder tab" },
    { id: "tab:moveRight", label: "Move Tab Right", category: "Tab", icon: "arrow-right", keywords: "reorder tab" },
    { id: "tab:goto:last", label: "Go to Last Tab", category: "Tab", icon: "arrow-right-to-bracket", keywords: "tab end" },
    { id: "pane:focus:next", label: "Next Pane", category: "Pane", icon: "arrow-right", keywords: "focus cycle forward" },
    { id: "pane:focus:prev", label: "Previous Pane", category: "Pane", icon: "arrow-left", keywords: "focus cycle backward" },
    { id: "pane:swap:up", label: "Swap Pane Up", category: "Pane", icon: "arrow-up", keywords: "move exchange neighbour above" },
    { id: "pane:swap:down", label: "Swap Pane Down", category: "Pane", icon: "arrow-down", keywords: "move exchange neighbour below" },
    { id: "pane:swap:left", label: "Swap Pane Left", category: "Pane", icon: "arrow-left", keywords: "move exchange neighbour" },
    { id: "pane:swap:right", label: "Swap Pane Right", category: "Pane", icon: "arrow-right", keywords: "move exchange neighbour" },
    { id: "pane:resize:up", label: "Move Pane Border Up", category: "Pane", icon: "arrows-up-down", keywords: "resize taller shorter" },
    { id: "pane:resize:down", label: "Move Pane Border Down", category: "Pane", icon: "arrows-up-down", keywords: "resize taller shorter" },
    { id: "pane:resize:left", label: "Move Pane Border Left", category: "Pane", icon: "arrows-left-right", keywords: "resize wider narrower" },
    { id: "pane:resize:right", label: "Move Pane Border Right", category: "Pane", icon: "arrows-left-right", keywords: "resize wider narrower" },
    { id: "pane:refocus", label: "Refocus Pane", category: "Pane", icon: "crosshairs", keywords: "focus keyboard caret" },
    { id: "pane:replaceWithLauncher", label: "Replace Pane with Shortcuts", category: "Pane", icon: "table-cells-large", keywords: "launcher empty pane replace" },
    { id: "pane:changeConnection", label: "Change Connection", category: "Pane", icon: "plug", keywords: "ssh remote wsl host connection" },
    { id: "pane:find", label: "Find in Pane", category: "Pane", icon: "magnifying-glass", keywords: "search text" },
    { id: "pane:voice", label: "Voice Input", category: "Pane", icon: "microphone", keywords: "dictate speak talk" },
    { id: "term:multiInput", label: "Type into All Terminals", category: "Pane", icon: "keyboard", keywords: "broadcast multi input terminals" },
    { id: "view:zoom:resetAll", label: "Reset Zoom on All Panes", category: "View", icon: "magnifying-glass", keywords: "actual size zoom every pane" },
    { id: "agent:focusComposer", label: "Focus the Message Box", category: "Agent", icon: "pen-to-square", keywords: "composer input type", view: "agent" },
    { id: "dev:perfHud", label: "Performance HUD", category: "Dev", icon: "gauge", keywords: "fps frames perf overlay", dev: true },
    { id: "dev:diagnostics", label: "Diagnostics Panel", category: "Dev", icon: "stethoscope", keywords: "diagnostics dispatch keys debug", dev: true },
];

/** Table commands with no palette entry, each with the reason. A row in neither list fails the coverage test. */
export const KEY_COMMANDS_WITHOUT_ENTRY: Readonly<Record<string, string>> = {
    "app:escape": "Escape closes whatever is open; it is a key, not an action to run by name",
    ...Object.fromEntries(
        [1, 2, 3, 4, 5, 6, 7, 8].map((n) => [`tab:goto:${n}`, "numbered; Next Tab, Previous Tab and Go to Last Tab cover them"])
    ),
    ...Object.fromEntries(
        [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`pane:focus:${n}`, "numbered; Focus Pane Up, Down, Left, Right and Next Pane cover them"])
    ),
};

/** Run a table command as its key would: the dispatcher's handler for a global one. */
async function runGlobal(command: string): Promise<void> {
    const { keyCommands } = await import("@/app/store/keymodel-dispatch");
    keyCommands.get(command)?.({ type: "keydown", key: "", code: "", repeat: false } as MuxKeyboardEvent);
}

/** Run a pane's own table command in the focused pane (its runner, as for MuxAPI's RunCommand). */
async function runInFocusedPane(command: string): Promise<void> {
    const api = (window as unknown as { __agentmux_shortcuts?: { run: (c: string) => unknown } }).__agentmux_shortcuts;
    await api?.run(command);
}

function specEntry(spec: KeyCommandSpec): CommandEntry {
    const entry: CommandEntry = {
        id: spec.id,
        label: spec.label,
        category: spec.category,
        icon: spec.icon,
        keywords: spec.keywords,
        execute: () => runGlobal(spec.id),
    };
    if (spec.dev) entry.when = devModeOn;
    if (spec.view) entry.when = () => focusedView() === spec.view;
    return entry;
}

const PANE_CATEGORY: Record<KeyPane, string> = { browser: "Browser", doctabs: "Documents", editor: "Editor", files: "Files" };

/** Whether the focused pane is one a table row of this kind reaches. */
function paneHasFocus(pane: KeyPane | "term"): boolean {
    const view = focusedView();
    if (!view) return false;
    return pane === "doctabs" ? DOC_TAB_HOSTS.includes(view) : pane === "term" ? view === "term" : view === pane;
}

/** The pane-scoped rows (and the terminal's own), once per command, shown while their pane has focus. */
function paneRowEntries(): CommandEntry[] {
    const seen = new Set<string>(KEY_COMMANDS.map((s) => s.id));
    const out: CommandEntry[] = [];
    for (const row of DEFAULT_KEYBINDINGS) {
        const pane: KeyPane | "term" | undefined = row.pane ?? (row.command.startsWith("term:") ? "term" : undefined);
        if (!pane || seen.has(row.command)) continue;
        seen.add(row.command);
        out.push(paneRowEntry(row, pane));
    }
    return out;
}

function paneRowEntry(row: KeyBindingRow, pane: KeyPane | "term"): CommandEntry {
    const category = pane === "term" ? "Terminal" : PANE_CATEGORY[pane];
    return {
        id: row.command,
        label: row.label,
        category,
        keywords: `${category} ${row.label}`,
        when: () => paneHasFocus(pane),
        execute: () => runInFocusedPane(row.command),
    };
}

/** Backdrop… and Bundles: entries the pane menu and the Memory pane had alone. */
function placeEntries(): CommandEntry[] {
    return [
        {
            id: "pane:backdrop",
            label: "Backdrop…",
            category: "Pane",
            icon: "fill-drip",
            keywords: "background colour color image theme pane backdrop",
            when: () => supportsBackdrop(focusedView()),
            execute: () => openBackdropPicker(getFocusedBlockId()),
        },
        {
            id: "open:bundles",
            label: "Open Bundles",
            category: "Open",
            icon: "box-archive",
            keywords: "bundle agents import export abf memory",
            execute: () => void openMemory("bundles"),
        },
    ];
}

/** Every entry this file adds: the global key commands, the pane-scoped rows, Backdrop… and Bundles. */
export function keyCommandEntries(): CommandEntry[] {
    return [...KEY_COMMANDS.map(specEntry), ...paneRowEntries(), ...placeEntries()];
}
