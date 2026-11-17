// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The ink of a backdrop (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §3): from one surface colour, every
 * foreground colour that sits on it. One pure function serves the built-ins, a custom colour the user picks and
 * an image's solved scrim, so contrast is a property of the function and `ink.test.ts` proves it, not a hope.
 *
 * The search runs in OKLCH, where moving lightness by the same amount reads the same at every hue, and every
 * candidate is judged by the WCAG 2.x ratio on the actual `#rrggbb` it would paint (`contrastRatio`), because that
 * is the standard we hold to (spec §12).
 */

import { contrastRatio } from "../block/pane-color-scheme";
import { hexToRgb, oklchOfHex, oklchToHex, type Oklch } from "../theme/oklch";

/** The app's status and accent hues, tone-moved (never re-hued) to read on a surface. */
export interface Signals {
    accent: string;
    error: string;
    warning: string;
    success: string;
    info: string;
}

/** The hues the default app theme paints (theme.scss: `--accent-color`, `--error-color`, ...). */
export const APP_SIGNALS: Signals = {
    accent: "#419fe0", // colour: fixed (the app's own hue, an input to the derivation)
    error: "#e54d2e", // colour: fixed (the app's own hue, an input to the derivation)
    warning: "#e0b956", // colour: fixed (the app's own hue, an input to the derivation)
    success: "#4e9a06", // colour: fixed (the app's own hue, an input to the derivation)
    info: "#419fe0", // colour: fixed (the app's own hue, an input to the derivation)
};

/**
 * What each ink token is held to, as a WCAG ratio against the hardest surface it can sit on. `aim` is what the
 * derivation reaches when the surface allows it; `floor` is what `ink.test.ts` fails below.
 */
export const INK_TARGETS = {
    text: { aim: 11, floor: 7 },
    muted: { aim: 6, floor: 4.5 },
    faint: { aim: 3.4, floor: 3 },
    border: { aim: 3.4, floor: 3 },
    divider: { aim: 1.7, floor: 1.5 },
    signal: { aim: 5, floor: 4.5 },
} as const;

export interface Ink {
    text: string;
    muted: string;
    faint: string;
    border: string;
    divider: string;
    accent: string;
    error: string;
    warning: string;
    success: string;
    info: string;
    focus: string;
    /** The ink at 8 percent, as `#rrggbbaa`: a wash over the surface. */
    hover: string;
    /** The ink at 16 percent. */
    selected: string;
}

export interface Surfaces {
    surface: string;
    /** Cards and inputs. */
    raised: string;
    /** Menus, popovers and toasts the pane opens. */
    overlay: string;
}

export interface DerivedBackdrop {
    /** `color-scheme` for native controls inside the pane: the side the surface is on. */
    scheme: "dark" | "light";
    surfaces: Surfaces;
    ink: Ink;
    /** `high`: primary text reaches 7:1. `readable`: it reaches 4.5:1 only (spec §3.5, rule 3). */
    level: "high" | "readable";
    /** The surface was moved a little in lightness so text can reach 7:1 (spec §3.5, rule 2). */
    nudged: boolean;
    /** The colour asked for; `surfaces.surface` differs from it only when `nudged`. */
    chosen: string;
    /** Ink tokens that miss their floor on the hardest surface. Empty for every colour the sweep tries. */
    unmet: string[];
}

const WHITE = "#ffffff";
const BLACK = "#000000";
/** The largest lightness the nudge may move a custom colour by. */
const NUDGE_CAP = 0.08;
const NUDGE_STEP = 0.004;
/** A little above the target, so a hex rounded to 8 bits still clears it. */
const MARGIN = 0.02;
/** How much of the ink a selected row washes over its surface (the hover wash is half of it). */
const SELECTED_WASH = 0.16;
/** Primary text keeps this ratio on a washed row, whatever level the colour reaches. */
const WASHED_TEXT_FLOOR = 4.5;

const clamp01 = (x: number) => Math.min(1, Math.max(0, x));

/** The raised and overlay layers sit a step further from the ink: lighter on a dark surface, darker on a pale one. */
function layersOf(base: Oklch, inkIsLight: boolean): Surfaces {
    const [up, over] = inkIsLight ? [0.03, 0.06] : [-0.02, -0.04];
    return {
        surface: oklchToHex(base.l, base.c, base.h),
        raised: oklchToHex(clamp01(base.l + up), base.c, base.h),
        overlay: oklchToHex(clamp01(base.l + over), base.c, base.h),
    };
}

/**
 * The surface the ink has the least contrast with: the lightest layer for light ink (or the darkest for dark ink).
 * Holding the ink to this one holds it to all three layers. (The wash of a hovered or selected row is checked on its
 * own, by the test: it only has to keep primary text at 4.5:1.)
 */
function hardestOf(layers: Surfaces, inkIsLight: boolean): string {
    const extreme = inkIsLight ? WHITE : BLACK;
    return Object.values(layers).sort((a, b) => contrastRatio(extreme, a) - contrastRatio(extreme, b))[0];
}

interface Solved {
    hex: string;
    ratio: number;
}

/**
 * The colour with `hue` and `chroma` that is nearest `from` in lightness and reaches `aim` against `against`,
 * moving toward white (`light`) or black. When `aim` is out of reach it is the farthest one, the most contrast
 * there is.
 */
function solveTone(against: string, light: boolean, hue: number, chroma: number, aim: number, from: number): Solved {
    const end = light ? 1 : 0;
    const at = (t: number) => oklchToHex(from + (end - from) * t, chroma, hue);
    const far = at(1);
    if (contrastRatio(far, against) < aim + MARGIN) return { hex: far, ratio: contrastRatio(far, against) };
    let [lo, hi] = [0, 1];
    for (let i = 0; i < 28; i++) {
        const mid = (lo + hi) / 2;
        if (contrastRatio(at(mid), against) >= aim + MARGIN) hi = mid;
        else lo = mid;
    }
    const hex = at(hi);
    return { hex, ratio: contrastRatio(hex, against) };
}

/** The most contrast pure white or black gets against `against`, on the ink's side. */
function bestPossible(against: string, light: boolean): number {
    return contrastRatio(light ? WHITE : BLACK, against);
}

/**
 * Move the surface's lightness away from the ink until the ink can reach `target` on the hardest layer. Primary text
 * is asked for 7:1 first, within the cap. A colour the cap cannot rescue is "readable" (spec §3.5, rule 3): it is
 * moved only as far as 4.5:1 needs, which every token's floor depends on, however far that is (the middle of the
 * dead zone is about 0.01 of lightness from the 4.5:1 crossover on each side, the layers' steps a little more).
 */
function nudgeSurface(base: Oklch, light: boolean): { base: Oklch; high: boolean } {
    // The wash of a selected row pulls the surface toward the ink, so the best ink must keep 4.5:1 on the hardest
    // layer under it too (the washed text is the pale-on-pale case a "readable" colour would otherwise miss).
    const reach = (b: Oklch, target: number) => {
        const hard = hardestOf(layersOf(b, light), light);
        const washed = washedOver(hard, light ? WHITE : BLACK, SELECTED_WASH);
        return bestPossible(hard, light) >= target + MARGIN && bestPossible(washed, light) >= WASHED_TEXT_FLOOR + MARGIN;
    };
    const step = (moved: number): Oklch => ({ ...base, l: clamp01(base.l + (light ? -moved : moved)) });
    if (reach(base, INK_TARGETS.text.floor)) return { base, high: true };
    for (let moved = NUDGE_STEP; moved <= NUDGE_CAP + 1e-9; moved += NUDGE_STEP) {
        if (reach(step(moved), INK_TARGETS.text.floor)) return { base: step(moved), high: true };
    }
    for (let moved = 0; moved <= 1; moved += NUDGE_STEP) {
        if (reach(step(moved), INK_TARGETS.muted.floor)) return { base: step(moved), high: false };
    }
    return { base, high: false };
}

/**
 * `surface` with `ink` washed over it by `t`: what a hovered or selected row paints, as `#rrggbb`. The alpha is the
 * 8-bit one `withAlpha` writes into the token (16 percent is 41/255), so the check judges the painted colour.
 */
function washedOver(surface: string, ink: string, t: number): string {
    const [a, b] = [hexToRgb(surface), hexToRgb(ink.slice(0, 7))];
    if (!a || !b) return surface;
    const ch = (i: number) =>
        Math.round((a[i] + (b[i] - a[i]) * (Math.round(t * 255) / 255)) * 255)
            .toString(16)
            .padStart(2, "0");
    return `#${ch(0)}${ch(1)}${ch(2)}`;
}

function withAlpha(hex: string, alpha: number): string {
    return `${hex}${Math.round(alpha * 255)
        .toString(16)
        .padStart(2, "0")}`;
}

/** A status or accent colour kept as it is when it already reads, otherwise moved in lightness only (hue is kept). */
function toneSignal(hex: string, against: string, light: boolean): string {
    const o = oklchOfHex(hex);
    if (!o) return hex;
    if (contrastRatio(hex, against) >= INK_TARGETS.signal.aim + MARGIN) return hex;
    return solveTone(against, light, o.h, o.c, INK_TARGETS.signal.aim, o.l).hex;
}

/** What `toneMove` did to a colour. */
export interface ToneMoved {
    /** The colour to use: `hex` itself when it already reads, else the same hue and chroma moved in lightness. */
    hex: string;
    /** The WCAG ratio of `hex` on the surface. */
    ratio: number;
    /** `ratio` reaches the target. False only when no lightness of this hue does (the farthest one is returned). */
    met: boolean;
    /** The colour was changed. */
    moved: boolean;
}

/**
 * A colour made to read on a surface (spec §6, the transcript's syntax and status colours): kept as it is when it
 * already reaches `target` against `surface`, otherwise moved in OKLCH lightness only, keeping its hue and chroma,
 * until the WCAG ratio on the real `#rrggbb` reaches `target` (with `solveTone`'s small margin, so a hex rounded to
 * 8 bits still clears it). It moves toward whichever of white and black the surface has more contrast with. When
 * even that end falls short, the farthest colour is returned with `met: false`, so the caller can say so.
 */
export function toneMove(hex: string, surface: string, target: number): ToneMoved {
    const o = oklchOfHex(hex);
    const have = contrastRatio(hex, surface);
    if (!o || have >= target) return { hex, ratio: have, met: have >= target, moved: false };
    const light = bestPossible(surface, true) >= bestPossible(surface, false);
    const solved = solveTone(surface, light, o.h, o.c, target, o.l);
    return { hex: solved.hex, ratio: solved.ratio, met: solved.ratio >= target, moved: solved.hex !== hex };
}

/**
 * Every ink for a surface. `signals` are the app's status hues (default: the default theme's).
 * Deterministic and pure: the same colour always gives the same backdrop, so it is stored as the colour alone.
 */
export function deriveInk(chosen: string, signals: Signals = APP_SIGNALS): DerivedBackdrop {
    const start = oklchOfHex(chosen) ?? { l: 0.22, c: 0, h: 0 };
    const light = bestPossible(oklchToHex(start.l, start.c, start.h), true) >= bestPossible(oklchToHex(start.l, start.c, start.h), false);
    const nudge = nudgeSurface(start, light);
    const layers = layersOf(nudge.base, light);
    const hard = hardestOf(layers, light);
    const { h, c } = nudge.base;
    const from = oklchOfHex(hard)!.l;
    const text = solveTone(hard, light, h, Math.min(c, 0.012), INK_TARGETS.text.aim, from).hex;
    const muted = solveTone(hard, light, h, Math.min(c, 0.02), INK_TARGETS.muted.aim, from).hex;
    const faint = solveTone(hard, light, h, Math.min(c, 0.02), INK_TARGETS.faint.aim, from).hex;
    const border = solveTone(hard, light, h, c, INK_TARGETS.border.aim, from).hex;
    const divider = solveTone(hard, light, h, c, INK_TARGETS.divider.aim, from).hex;
    const tone = (hex: string) => toneSignal(hex, hard, light);
    const accent = tone(signals.accent);
    const ink: Ink = {
        text,
        muted,
        faint,
        border,
        divider,
        accent,
        error: tone(signals.error),
        warning: tone(signals.warning),
        success: tone(signals.success),
        info: tone(signals.info),
        focus: accent,
        hover: withAlpha(text, 0.08),
        selected: withAlpha(text, SELECTED_WASH),
    };
    const onSelected = washedOver(hard, text, SELECTED_WASH);
    const checks: [string, string, number, string?][] = [
        ["text", ink.text, nudge.high ? INK_TARGETS.text.floor : 4.5],
        ["textOnSelected", ink.text, WASHED_TEXT_FLOOR, onSelected],
        ["muted", ink.muted, INK_TARGETS.muted.floor],
        ["faint", ink.faint, INK_TARGETS.faint.floor],
        ["border", ink.border, INK_TARGETS.border.floor],
        ["divider", ink.divider, INK_TARGETS.divider.floor],
        ["accent", ink.accent, INK_TARGETS.signal.floor],
        ["error", ink.error, INK_TARGETS.signal.floor],
        ["warning", ink.warning, INK_TARGETS.signal.floor],
        ["success", ink.success, INK_TARGETS.signal.floor],
        ["info", ink.info, INK_TARGETS.signal.floor],
    ];
    return {
        scheme: light ? "dark" : "light",
        surfaces: layers,
        ink,
        level: nudge.high ? "high" : "readable",
        nudged: nudge.base.l !== start.l,
        chosen,
        unmet: checks.filter(([, hex, floor, on]) => contrastRatio(hex, on ?? hard) < floor).map(([name]) => name),
    };
}
