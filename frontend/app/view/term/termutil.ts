// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const DefaultTermTheme = "default-dark";
import { colord } from "colord";

// Built-in high-contrast palette used when the backend `wconfig` doesn't
// supply a `termthemes` table (which it currently doesn't — see
// agentmux-srv/src/backend/wconfig/mod.rs). Without this fallback every
// xterm in the app would render with xterm.js's library defaults, which
// include dim greys for the ANSI palette and a near-white foreground
// that looks washed out on dark panel backgrounds.
//
// This is *not* a competing source of truth — `computeTheme` still
// prefers `fullConfig.termthemes[themeName]` when present. The fallback
// only fires when neither the requested theme nor the configured
// default exists, which is the steady state today.
const FALLBACK_TERM_THEME: TermThemeType = {
    "display:name": "Built-in dark",
    "display:order": 0,
    cmdText: "#f8f8f2",
    foreground: "#f8f8f2",
    background: "#0d0e0f",
    cursor: "#f8f8f2",
    cursorAccent: "#0d0e0f",
    selectionBackground: "#44475a",
    black: "#21222c",
    red: "#ff5555",
    green: "#50fa7b",
    yellow: "#f1fa8c",
    blue: "#bd93f9",
    magenta: "#ff79c6",
    cyan: "#8be9fd",
    white: "#f8f8f2",
    brightBlack: "#6272a4",
    brightRed: "#ff6e6e",
    brightGreen: "#69ff94",
    brightYellow: "#ffffa5",
    brightBlue: "#d6acff",
    brightMagenta: "#ff92df",
    brightCyan: "#a4ffff",
    brightWhite: "#ffffff",
} as TermThemeType;

function applyTransparencyToColor(hexColor: string, transparency: number): string {
    const alpha = 1 - transparency; // transparency is already 0-1
    return colord(hexColor).alpha(alpha).toHex();
}

// returns (theme, bgcolor, transparency (0 - 1.0))
function computeTheme(
    fullConfig: FullConfigType,
    themeName: string,
    termTransparency: number
): [TermThemeType, string] {
    let theme: TermThemeType = fullConfig?.termthemes?.[themeName];
    if (theme == null) {
        theme = fullConfig?.termthemes?.[DefaultTermTheme] || FALLBACK_TERM_THEME;
    }
    const themeCopy = { ...theme };
    if (termTransparency != null && termTransparency > 0) {
        if (themeCopy.background) {
            themeCopy.background = applyTransparencyToColor(themeCopy.background, termTransparency);
        }
        if (themeCopy.selectionBackground) {
            themeCopy.selectionBackground = applyTransparencyToColor(themeCopy.selectionBackground, termTransparency);
        }
    }
    let bgcolor = themeCopy.background;
    themeCopy.background = "#00000000";
    return [themeCopy, bgcolor];
}

/**
 * Resolve the terminal theme for callers without a blockId (modals,
 * install dialogs, anything outside the pane tree). Respects the global
 * `term:theme` setting with a fallback to DefaultTermTheme. Skips
 * transparency — callers in this position render on opaque panel
 * backgrounds where transparency is meaningless.
 *
 * See docs/specs/SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18.md.
 */
function computeTermThemeFromSettings(fullConfig: FullConfigType): [TermThemeType, string] {
    const themeName = fullConfig?.settings?.["term:theme"] ?? DefaultTermTheme;
    const [theme, bgcolor] = computeTheme(fullConfig, themeName, 0);
    // Modal callers paint xterm directly onto the panel surface — restore
    // the resolved background so xterm renders the theme's bg rather than
    // letting the container CSS's hardcoded color bleed through. Block-pane
    // callers want the original behavior (transparent bg → blockBg shows
    // through), so `computeTheme` stays unchanged. Codex caught this on PR
    // #895: a non-dark theme would put light foreground on a hardcoded
    // dark container bg.
    if (bgcolor) {
        theme.background = bgcolor;
    }
    return [theme, bgcolor];
}

export { computeTheme, computeTermThemeFromSettings };
