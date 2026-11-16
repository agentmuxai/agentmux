// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The active app theme's terminal palette, as the transcript reads it (spec §6). Every app theme sets its own
 * `--term-*` colours, so a pane's backdrop cannot carry a table of them: it reads the ones in effect and moves each
 * to read on the pane (`scopeVars`). The read is shared by every pane, done when something first asks and again
 * only after the theme changes, never per render or per frame.
 */

import { createSignal } from "solid-js";
import { hexToRgb } from "../theme/oklch";
import mapped from "./mapped-tokens.json";

/** The `--term-*` tokens the agent pane's transcript reads as syntax and status colours. */
export const ANSI_TOKENS: ReadonlyArray<string> = mapped.ansi;

/** A token's value as `#rrggbb`, by token. A token the theme does not set as a hex is left out. */
export type AnsiColours = Record<string, string>;

/** `#rgb` or `#rrggbb` as `#rrggbb`, or undefined for any other form (a name, `rgb()`), which is left as the theme has it. */
function plainHex(value: string): string | undefined {
    const v = value.trim();
    const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(v);
    const hex = short ? `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}` : v;
    return /^#[0-9a-f]{6}$/i.test(hex) && hexToRgb(hex) ? hex.toLowerCase() : undefined;
}

/** The transcript's palette as `el` has it now: one style read of the nine tokens. */
export function readAnsi(el: Element = document.documentElement): AnsiColours {
    const style = getComputedStyle(el);
    const out: AnsiColours = {};
    for (const token of ANSI_TOKENS) {
        const hex = plainHex(style.getPropertyValue(token));
        if (hex) out[token] = hex;
    }
    return out;
}

/** The attributes the app sets on `<html>` when the theme changes. */
const THEME_ATTRIBUTES = ["data-theme", "data-theme-polarity", "class"];

const [stamp, bump] = createSignal(0);
let current: AnsiColours | undefined;
let observer: MutationObserver | undefined;

function watchTheme(): void {
    if (observer || typeof MutationObserver === "undefined" || typeof document === "undefined") return;
    observer = new MutationObserver(() => {
        current = undefined;
        bump((n) => n + 1);
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: THEME_ATTRIBUTES });
}

/**
 * The active theme's palette, read on first use and again after the theme changes. Reactive: an effect that reads it
 * runs again then. A pane with no backdrop never calls it, so no read is made and nothing is watched for it.
 */
export function themeAnsi(): AnsiColours {
    stamp();
    watchTheme();
    return (current ??= readAnsi());
}

/** For tests: forget the cached palette and stop watching. Effects that read it run once more, and what they read is forgotten too. */
export function resetThemeAnsi(): void {
    observer?.disconnect();
    observer = undefined;
    bump((n) => n + 1);
    current = undefined;
}
