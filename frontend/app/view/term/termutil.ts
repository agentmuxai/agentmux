// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const DefaultTermTheme = "default-dark";
import { colord } from "colord";

// Last-resort palette for when `document`/CSS custom properties aren't
// available at all (e.g. a non-browser test environment) — see
// `tryDeriveTermThemeFromCss` below, which is what actually fires in the
// steady state today (no `termthemes` table configured — see
// agentmux-srv/src/backend/wconfig/mod.rs). Without either fallback, every
// xterm in the app would render with xterm.js's library defaults, which
// include dim greys for the ANSI palette and a near-white foreground that
// looks washed out on dark panel backgrounds.
//
// This is *not* a competing source of truth — `computeTheme` still prefers
// `fullConfig.termthemes[themeName]` when present, then the CSS-derived
// theme. This literal only fires when all of those miss.
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
} as unknown as TermThemeType;

function applyTransparencyToColor(hexColor: string, transparency: number): string {
    const alpha = 1 - transparency; // transparency is already 0-1
    return colord(hexColor).alpha(alpha).toHex();
}

// Builds a TermThemeType from the currently-active app theme's --term-*
// custom properties (theme.scss + frontend/app/themes/*.scss), so a
// terminal with no explicit term:theme picks up whatever window:theme is
// active instead of a fixed, theme-independent palette. See
// SPEC_TERMINAL_THEME_PENETRATION_2026_07_07.md.
//
// No dedicated --term-cursor token exists (only --term-cursor-accent) —
// cursor reuses --term-foreground, the common "cursor matches text color"
// convention, rather than adding a new token across every theme file for a
// value that would almost always equal foreground anyway.
//
// cursorAccent isn't part of TermThemeType's wire shape (frontend/types/
// gotypes.d.ts) but IS a real xterm.js ITheme option — same tolerant `as
// unknown as TermThemeType` cast FALLBACK_TERM_THEME below already uses for
// the same reason, not a typo.
function tryDeriveTermThemeFromCss(): TermThemeType | null {
    if (typeof document === "undefined") return null;
    // Only runs from computeTheme, invoked on mount and on theme/settings
    // changes (an infrequent, deliberate user action), never per-keystroke.
    const style = getComputedStyle(document.documentElement); // perf:allow-layout-read — theme switch, not input-handler hot path
    const v = (name: string) => style.getPropertyValue(name).trim();
    const background = v("--term-background");
    const foreground = v("--term-foreground");
    if (!background || !foreground) return null; // theme.scss not loaded / not a real DOM
    const themeName = document.documentElement.getAttribute("data-theme") || "default";
    return {
        "display:name": `App theme (${themeName})`,
        "display:order": -1,
        black: v("--term-black"),
        red: v("--term-red"),
        green: v("--term-green"),
        yellow: v("--term-yellow"),
        blue: v("--term-blue"),
        magenta: v("--term-magenta"),
        cyan: v("--term-cyan"),
        white: v("--term-white"),
        brightBlack: v("--term-bright-black"),
        brightRed: v("--term-bright-red"),
        brightGreen: v("--term-bright-green"),
        brightYellow: v("--term-bright-yellow"),
        brightBlue: v("--term-bright-blue"),
        brightMagenta: v("--term-bright-magenta"),
        brightCyan: v("--term-bright-cyan"),
        brightWhite: v("--term-bright-white"),
        gray: v("--term-gray"),
        cmdtext: v("--term-cmdtext"),
        foreground,
        background,
        selectionBackground: v("--term-selection-background"),
        cursor: foreground,
        cursorAccent: v("--term-cursor-accent"),
    } as unknown as TermThemeType;
}

// returns (theme, bgcolor, transparency (0 - 1.0))
function computeTheme(
    fullConfig: FullConfigType,
    themeName: string,
    termTransparency: number
): [TermThemeType, string] {
    let theme: TermThemeType = fullConfig?.termthemes?.[themeName];
    if (theme == null) {
        theme = fullConfig?.termthemes?.[DefaultTermTheme] || tryDeriveTermThemeFromCss() || FALLBACK_TERM_THEME;
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

export { computeTheme, computeTermThemeFromSettings, tryDeriveTermThemeFromCss };
