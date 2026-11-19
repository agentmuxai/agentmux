// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a picked palette colour paints, per surface and state, in one place
 * (docs/specs/SPEC_PICKED_COLOUR_IS_THE_COLOUR_SEEN_2026_11_18.md §5). A surface asks here; the picker draws its
 * sample from the same call, so what is picked is what is seen. Today the window tab is here; the other surfaces
 * move in by phase.
 *
 * Every state is a tone of its own, not a `filter` or an overlay on another one: hover is one small lightness step
 * from the resting tone, toward the theme's text (lighter on a dark theme, darker on a light one). A fill that
 * cannot carry its label at 4.5:1 is moved in lightness only, hue and chroma kept, and the sample shows that colour.
 */

import { contrastRatio, slotRoleColor, type OklchTone } from "../block/pane-color-scheme";
import { oklchOfHex, oklchToHex } from "./oklch";
import type { PaletteSlot } from "./palette";

type Theme = "dark" | "light";

/** The colours a window tab's label is drawn in: the selected tab's and the others'. */
export interface TabInk {
    selected: string;
    unselected: string;
}

/** The default dark and light themes' text tokens (`--main-text-color`, `--secondary-text-color`), which tab.scss uses. */
export const DEFAULT_TAB_INK: Record<Theme, TabInk> = {
    dark: {
        selected: "#f7f7f7", // colour: fixed (the default dark theme's --main-text-color, the fallback when a theme's cannot be read)
        unselected: "#c3c8c2", // colour: fixed (the default dark theme's --secondary-text-color, the same fallback)
    },
    light: {
        selected: "#0d0f11", // colour: fixed (the light theme's --main-text-color, the fallback when a theme's cannot be read)
        unselected: "#4b535c", // colour: fixed (the light theme's --secondary-text-color, the same fallback)
    },
};

/** A label on its fill: the WCAG floor for normal text. */
export const LABEL_FLOOR = 4.5;
/** The bar on the selected fill: the WCAG floor for a graphical object. */
export const BAR_FLOOR = 3;
/** The close mark on a fill: the same floor, it is a graphical object. */
export const CLOSE_FLOOR = 3;
/** What the bar is drawn to, a little over the floor so an 8-bit rounding cannot take it under. */
const BAR_TARGET = 3.15;

/**
 * The OKLCH tones of a hue's window tab. The unselected fill is the quiet one; the selected fill is a step stronger,
 * so selection reads by lightness and chroma and by the bar, not by hue alone. `hoverStep` is the lightness step of
 * a hovered tab.
 */
export const TAB_TONES: Record<Theme, { selected: OklchTone; unselected: OklchTone; hoverStep: number }> = {
    dark: { selected: { l: 0.52, c: 0.13 }, unselected: { l: 0.33, c: 0.06 }, hoverStep: 0.03 },
    light: { selected: { l: 0.78, c: 0.13 }, unselected: { l: 0.91, c: 0.04 }, hoverStep: -0.03 },
};

/** What a window tab paints for a slot in a theme, as `#rrggbb`. */
export interface TabPaint {
    unselected: string;
    unselectedHover: string;
    selected: string;
    selectedHover: string;
    /** The bar on the selected tab. */
    bar: string;
}

const clamp01 = (x: number) => Math.min(1, Math.max(0, x));

/** The colour at lightness `l` of `hex`'s hue and chroma. */
function withLightness(hex: string, l: number): string {
    const o = oklchOfHex(hex);
    return o ? oklchToHex(clamp01(l), o.c, o.h) : hex;
}

/** A colour a fill has to hold a contrast ratio against: its label, or the close mark. */
interface Check {
    against: string;
    floor: number;
}

/**
 * `hex` moved in OKLCH lightness only (hue and chroma kept), toward lighter (`1`) or darker (`-1`), until it passes every
 * check. The same colour when it already does; the end of the scale when none does.
 */
export function moveTone(hex: string, checks: Check[], direction: 1 | -1): string {
    const o = oklchOfHex(hex);
    const holds = (c: string) => checks.every((k) => contrastRatio(c, k.against) >= k.floor);
    if (!o || holds(hex)) return hex;
    let last = hex;
    for (let l = o.l; l >= 0 && l <= 1; l += 0.005 * direction) {
        last = oklchToHex(clamp01(l), o.c, o.h);
        if (holds(last)) return last;
    }
    return last;
}

/** The side of its checks a fill moves to: away from the first one's colour (toward white from dark ink, toward black from light). */
function awayFrom(fill: string, checks: Check[]): 1 | -1 {
    return (oklchOfHex(checks[0].against)?.l ?? 0) > (oklchOfHex(fill)?.l ?? 0) ? -1 : 1;
}

/**
 * `fill`, moved in lightness away from its label until the label reads on it at `floor` (the same colour when it
 * already does): toward white when the label is dark, toward black when it is light.
 */
export function readableFill(fill: string, label: string, floor: number = LABEL_FLOOR): string {
    return fitFill(fill, [{ against: label, floor }]);
}

function fitFill(fill: string, checks: Check[]): string {
    return oklchOfHex(fill) ? moveTone(fill, checks, awayFrom(fill, checks)) : fill;
}

/** ΔE in OKLab, times 100: 1 to 2 is the least a person sees. */
export function deltaE(a: string, b: string): number {
    const [x, y] = [oklchOfHex(a), oklchOfHex(b)];
    if (!x || !y) return 0;
    const ab = (o: { c: number; h: number }) => [o.c * Math.cos((o.h * Math.PI) / 180), o.c * Math.sin((o.h * Math.PI) / 180)];
    const [p, q] = [ab(x), ab(y)];
    return Math.hypot(x.l - y.l, p[0] - q[0], p[1] - q[1]) * 100;
}

/** The least a hover moves a fill, as ΔE: a step a person can see, even at the dark end of the scale where a step in L is tiny. */
const MIN_HOVER_DELTA = 2.5;

/**
 * The hovered fill: a visible step from the resting one toward the theme's text; when that would take a check under its
 * floor, the same step the other way. So hover always moves, and always reads.
 */
function hoverFill(rest: string, checks: Check[], step: number): string {
    const l = oklchOfHex(rest)?.l ?? 0;
    const holds = (c: string) => checks.every((k) => contrastRatio(c, k.against) >= k.floor);
    for (const dir of [Math.sign(step), -Math.sign(step)]) {
        for (let size = Math.abs(step); size <= 0.15; size += 0.01) {
            const c = withLightness(rest, l + dir * size);
            if (deltaE(rest, c) < MIN_HOVER_DELTA) continue;
            if (holds(c)) return c;
            break;
        }
    }
    return fitFill(withLightness(rest, l + step), checks);
}

/**
 * The bar of the selected tab: the slot's identity tone, moved in lightness away from the fills it sits on (lighter on a
 * dark theme, darker on a light one, the side the strip behind it is on) until it holds `BAR_TARGET` against the selected
 * fill and its hover. So the bar is the picked hue, drawn so that it can be seen on the tab it marks.
 */
function barOn(slot: PaletteSlot, isLightTheme: boolean, fills: string[]): string {
    return moveTone(slotRoleColor(slot, isLightTheme, "identity"), fills.map((against) => ({ against, floor: BAR_TARGET })), isLightTheme ? -1 : 1);
}

/**
 * The OKLCH lightness of a window tab's fills for Black and White (no chroma). Black and White cannot share the hues'
 * tones: a black fill is the surface itself on a dark theme, and a white one is on a light theme. The selected and the
 * unselected fills stay far enough apart that the pair reads as a selection, the bar apart.
 */
const TAB_NEUTRAL_TONES: Record<Theme, Record<"black" | "white", { selected: number; unselected: number }>> = {
    dark: { black: { selected: 0, unselected: 0.16 }, white: { selected: 0.5, unselected: 0.42 } },
    light: { black: { selected: 0.72, unselected: 0.82 }, white: { selected: 1, unselected: 0.94 } },
};

function tabFills(slot: PaletteSlot, theme: Theme): { selected: string; unselected: string } {
    if (slot.neutral) {
        const tones = TAB_NEUTRAL_TONES[theme][slot.neutral];
        return { selected: oklchToHex(tones.selected, 0, 0), unselected: oklchToHex(tones.unselected, 0, 0) };
    }
    const t = TAB_TONES[theme];
    const hue = slot.hue ?? 0;
    return { selected: oklchToHex(t.selected.l, t.selected.c, hue), unselected: oklchToHex(t.unselected.l, t.unselected.c, hue) };
}

/** What a window tab of this slot paints in a theme, every fill readable under `ink`. */
export function tabPaint(slot: PaletteSlot, isLightTheme: boolean, ink?: TabInk): TabPaint {
    const theme: Theme = isLightTheme ? "light" : "dark";
    const labels = ink ?? DEFAULT_TAB_INK[theme];
    const base = tabFills(slot, theme);
    const step = TAB_TONES[theme].hoverStep;
    // The selected tab's label is the main text, and its close mark (shown on it always) the secondary text.
    const onSelected: Check[] = [{ against: labels.selected, floor: LABEL_FLOOR }, { against: labels.unselected, floor: CLOSE_FLOOR }];
    const onUnselected: Check[] = [{ against: labels.unselected, floor: LABEL_FLOOR }];
    const selected = fitFill(base.selected, onSelected);
    const unselected = fitFill(base.unselected, onUnselected);
    const selectedHover = hoverFill(selected, onSelected, step);
    return {
        selected,
        selectedHover,
        unselected,
        unselectedHover: hoverFill(unselected, onUnselected, step),
        bar: barOn(slot, isLightTheme, [selected, selectedHover]),
    };
}

/** The picker's swatch for a window tab: the selected tab's fill and its bar, what the tab will be when it is picked. */
export function tabSample(slot: PaletteSlot, isLightTheme: boolean, ink?: TabInk): { fill: string; bar: string } {
    const p = tabPaint(slot, isLightTheme, ink);
    return { fill: p.selected, bar: p.bar };
}
