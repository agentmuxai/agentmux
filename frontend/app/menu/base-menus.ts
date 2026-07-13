// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Theme list shown in the hamburger menu Theme submenu. Order matters
 * for muscle memory — don't reshuffle. Ids must match the schema enum
 * at schema/settings.json `window:theme`, and each theme's SCSS file
 * at frontend/app/themes/<id>.scss.
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
    { id: "light", label: "Light" },
    { id: "catppuccin-latte", label: "Catppuccin Latte" },
    { id: "solarized-light", label: "Solarized Light" },
    { id: "gruvbox-light", label: "Gruvbox Light" },
];

/**
 * Theme ids that are light-background (the inverse of every other theme's
 * dark-background convention). Consumed by app.tsx to additionally set
 * `data-theme-polarity="light"` on <html> alongside `data-theme="<id>"`.
 *
 * Why a separate polarity marker instead of gating CSS on the specific
 * theme id: a handful of chrome elements (e.g. the Windows caption-button
 * hover/press tint, window-controls.win32.scss) need a light-vs-dark
 * decision but don't have (and shouldn't need) per-theme tuning — one
 * `[data-theme-polarity="light"]` selector covers every light theme
 * without hardcoding N theme ids into N call sites, and covers whatever
 * light theme is added next for free.
 * See docs/specs/SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13.md.
 */
export const LIGHT_THEME_IDS: ReadonlySet<string> = new Set([
    "light",
    "catppuccin-latte",
    "solarized-light",
    "gruvbox-light",
]);

