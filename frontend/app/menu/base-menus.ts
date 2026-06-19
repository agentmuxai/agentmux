// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Theme list shown in the hamburger menu Theme submenu. Order matters
 * for muscle memory — don't reshuffle. Ids must match the schema enum
 * at schema/settings.json `window:theme`.
 */
export const THEME_OPTIONS: ReadonlyArray<{ id: string; label: string }> = [
    { id: "default", label: "Default" },
    { id: "midnight", label: "Midnight" },
    { id: "high-contrast", label: "High Contrast" },
    { id: "monokai", label: "Monokai" },
    { id: "nord", label: "Nord" },
    { id: "dracula", label: "Dracula" },
    { id: "catppuccin", label: "Catppuccin" },
    { id: "tokyo-night", label: "Tokyo Night" },
    { id: "gruvbox", label: "Gruvbox" },
];

