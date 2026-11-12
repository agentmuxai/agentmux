// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { settingEntry, type SettingsIndexEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const KEYBOARD_SETTINGS = {
    shortcuts: settingEntry({
        id: "keyboard.shortcuts",
        labelKey: "settings.keyboard.shortcuts.label",
        descriptionKey: "settings.keyboard.shortcuts.description",
        section: "keyboard",
        keywords: ["shortcut", "shortcuts", "keybinding", "keybindings", "key binding", "hotkey", "hotkeys", "keymap", "rebind", "keyboard", "keybindings.json"],
    }),
    resetAll: settingEntry({
        id: "keyboard.reset_all",
        labelKey: "settings.keyboard.resetAll.label",
        descriptionKey: "settings.keyboard.resetAll.description",
        section: "keyboard",
        keywords: ["reset keybindings", "restore default shortcuts", "default keys", "clear custom shortcuts"],
    }),
} satisfies Record<string, SettingsIndexEntry>;
