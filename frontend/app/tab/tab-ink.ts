// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TabInk } from "../theme/paint";

/** `#rrggbb` of a theme text token as written (`#f7f7f7`, `rgb(195, 200, 194)`), or undefined for any other form. */
export function parseInkToken(raw: string | undefined | null): string | undefined {
    const value = raw?.trim().toLowerCase();
    if (!value) return undefined;
    if (/^#[0-9a-f]{6}$/.test(value)) return value;
    const m = /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/.exec(value);
    return m ? "#" + [m[1], m[2], m[3]].map((n) => Math.min(255, Number(n)).toString(16).padStart(2, "0")).join("") : undefined;
}

/**
 * The colours the window tabs' labels are drawn in now: the theme's `--main-text-color` (the selected tab) and
 * `--secondary-text-color` (the others). Undefined when either is not a plain colour; paint.ts then uses the default theme's.
 */
export function currentTabInk(root: Element = document.documentElement): TabInk | undefined {
    const style = getComputedStyle(root);
    const selected = parseInkToken(style.getPropertyValue("--main-text-color"));
    const unselected = parseInkToken(style.getPropertyValue("--secondary-text-color"));
    return selected && unselected ? { selected, unselected } : undefined;
}
