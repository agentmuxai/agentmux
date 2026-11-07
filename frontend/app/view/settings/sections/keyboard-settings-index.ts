// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SettingsIndexEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const KEYBOARD_SETTINGS = {
    shortcuts: {
        id: "keyboard.shortcuts",
        label: "Keyboard shortcuts",
        description: "See every shortcut and change or remove the ones you want. Help shows your own keys.",
        section: "keyboard",
        keywords: ["shortcut", "shortcuts", "keybinding", "keybindings", "key binding", "hotkey", "hotkeys", "keymap", "rebind", "keyboard", "keybindings.json"],
    },
    resetAll: {
        id: "keyboard.reset_all",
        label: "Reset all shortcuts",
        description: "Put every shortcut back to the key AgentMux ships with.",
        section: "keyboard",
        keywords: ["reset keybindings", "restore default shortcuts", "default keys", "clear custom shortcuts"],
    },
} satisfies Record<string, SettingsIndexEntry>;
