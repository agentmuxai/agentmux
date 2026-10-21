// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const DefaultTermTheme = "default-dark";
import { detectAgentFromEnv } from "@/app/block/autotitle";

// Last-resort palette for when `document`/CSS custom properties aren't
// available at all (e.g. a non-browser test environment) — see
// `tryDeriveTermThemeFromCss` below, which is what actually fires in the
// steady state today (no `termthemes` table configured — see
// crates/srv/src/backend/wconfig/mod.rs). Without either fallback, every
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

/** `[r, g, b, a]` (0-255, alpha 0-1) of a CSS colour: hex (3, 4, 6 or 8
 *  digits) and `rgb()`/`rgba()` directly, anything else (`hsl()`, a name)
 *  through a canvas, which normalises every valid CSS colour. Null if it isn't
 *  one. */
function parseColor(color: string): [number, number, number, number] | null {
    const c = color.trim();
    const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(c);
    if (hex) {
        let h = hex[1];
        if (h.length <= 4) h = [...h].map((d) => d + d).join("");
        const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
        return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
    }
    const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:[\s,/]+([\d.]+%?))?\s*\)$/i.exec(c);
    if (rgb) {
        const a = rgb[4] == null ? 1 : rgb[4].endsWith("%") ? parseFloat(rgb[4]) / 100 : parseFloat(rgb[4]);
        return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3]), a];
    }
    if (typeof document === "undefined") return null;
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return null;
    // An invalid colour leaves fillStyle unchanged: tell it apart with a sentinel.
    ctx.fillStyle = "#010203";
    ctx.fillStyle = c;
    const normalized = String(ctx.fillStyle);
    if (normalized === "#010203" && !/^#010203$/i.test(c)) return null;
    return normalized.startsWith("#") || normalized.startsWith("rgb") ? parseColor(normalized) : null;
}

/** `color` with its alpha set to `1 - transparency`, as `#rrggbb[aa]`: what
 *  colord's `alpha(x).toHex()` gave, which replaced the colour's own alpha. A
 *  colour it can't read is returned as it is. */
function applyTransparencyToColor(color: string, transparency: number): string {
    const parsed = parseColor(color);
    if (!parsed) return color;
    const [r, g, b] = parsed;
    const alpha = Math.max(0, Math.min(1, 1 - transparency)); // transparency is already 0-1
    const hex = (v: number) => Math.round(v).toString(16).padStart(2, "0");
    return `#${hex(r)}${hex(g)}${hex(b)}${alpha === 1 ? "" : hex(alpha * 255)}`;
}

// Builds a TermThemeType from the currently-active app theme's --term-*
// custom properties (theme.scss + frontend/app/themes/*.scss), so a
// terminal with no explicit term:theme picks up whatever window:theme is
// active instead of a fixed, theme-independent palette (#2010).
//
// No dedicated --term-cursor token exists (only --term-cursor-accent) —
// cursor reuses --term-foreground, the common "cursor matches text color"
// convention, rather than adding a new token across every theme file for a
// value that would almost always equal foreground anyway.
//
// cursorAccent isn't part of TermThemeType's wire shape (frontend/types/
// srv-types.d.ts) but IS a real xterm.js ITheme option — same tolerant `as
// unknown as TermThemeType` cast FALLBACK_TERM_THEME above already uses for
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

export { applyTransparencyToColor, computeTheme, tryDeriveTermThemeFromCss };

/**
 * A terminal pane's header name: the agent it runs (from `cmd:env`), else
 * nothing for a `cmd` controller, else "Terminal". Lives with the terminal
 * since Pane Tab contract Phase 5 (it was a name check in BlockFrame).
 */
export function termViewName(meta: MetaType | undefined): string {
    const agentId = detectAgentFromEnv(meta?.["cmd:env"] as Record<string, string> | undefined);
    if (agentId) return agentId;
    if (meta?.controller == "cmd") return "";
    return "Terminal";
}
