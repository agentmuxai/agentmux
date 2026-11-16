// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's colour scope (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §7): the custom properties one pane
 * carries on its own element so everything inside it reads its backdrop, not the window's theme. `scopeVars` turns a
 * derived backdrop into those properties; the registry keeps each block's set in a signal so a pane's chrome and its
 * view see the same one; `PaneScope` (PaneScope.tsx) puts them on a div.
 *
 * The properties are the backdrop's own (`--surface*`, `--ink-*`, `color-scheme`) and the app's existing tokens,
 * re-pointed at them, so a stylesheet written for the theme works unchanged inside a pane. Aliases built on those
 * tokens are re-resolved for the pane by `_aliases.scss` (scripts/gen-backdrop-aliases.mjs).
 */

import { createSignal, type Accessor } from "solid-js";
import { hexToRgb } from "../theme/oklch";
import { contrastRatio } from "../block/pane-color-scheme";
import type { AnsiColours } from "./ansi";
import { toneMove, type DerivedBackdrop, type Surfaces } from "./ink";
import mapped from "./mapped-tokens.json";

export type ScopeVars = Record<string, string>;

/**
 * The existing tokens a pane re-points (mapped-tokens.json, which scripts/gen-backdrop-aliases.mjs reads too, §7.3):
 * what a window theme sets, and what a pane reads it from instead.
 */
export const SURFACE_TOKEN_MAP: Readonly<Record<string, string>> = mapped.surface;
/** The ink-coloured tokens: each is read from the backdrop property named here. */
export const INK_TOKEN_MAP: Readonly<Record<string, string>> = mapped.ink;

/** Every existing token a pane scope re-points: the generator of `_aliases.scss` starts its search from these. */
export const MAPPED_TOKENS: ReadonlyArray<string> = [...Object.keys(SURFACE_TOKEN_MAP), ...Object.keys(INK_TOKEN_MAP), ...mapped.channels];

/** `r, g, b` of a `#rrggbb`, the form `rgba(var(--accent-color-rgb), 0.2)` takes. */
function rgbTriplet(hex: string): string | undefined {
    const rgb = hexToRgb(hex.slice(0, 7));
    return rgb ? rgb.map((c) => Math.round(c * 255)).join(", ") : undefined;
}

/** What the transcript's terminal-palette colours are held to on a pane's surfaces (WCAG, as for muted text). */
export const ANSI_CONTRAST = 4.5;

/**
 * A palette colour moved, in lightness only, to read on all three of the pane's layers. The layer that is hardest for
 * the colour changes as it moves (a colour that clears the raised layer can still miss the overlay one), so it is
 * moved again against whichever layer it is worst on, until it reads on all of them or stops moving.
 */
function readableOn(hex: string, surfaces: Surfaces): string {
    const layers = [surfaces.surface, surfaces.raised, surfaces.overlay];
    let out = hex;
    for (let pass = 0; pass < layers.length + 1; pass++) {
        const worst = layers.reduce((a, b) => (contrastRatio(out, b) < contrastRatio(out, a) ? b : a));
        if (contrastRatio(out, worst) >= ANSI_CONTRAST) break;
        const next = toneMove(out, worst, ANSI_CONTRAST).hex;
        if (next === out) break;
        out = next;
    }
    return out;
}

/**
 * The custom properties for a pane's inline style, from what `deriveInk` made of its backdrop. `ansi` is the active
 * app theme's terminal palette (`themeAnsi`): each colour in it is set again on the pane, moved to read there, for the
 * transcript's syntax and status colours (spec §6). Without it the pane sets none, and they stay the theme's.
 */
export function scopeVars(derived: DerivedBackdrop, ansi?: AnsiColours): ScopeVars {
    const { surfaces, ink } = derived;
    const vars: ScopeVars = {
        "color-scheme": derived.scheme,
        "--surface": surfaces.surface,
        "--surface-raised": surfaces.raised,
        "--surface-overlay": surfaces.overlay,
        "--ink-text": ink.text,
        "--ink-muted": ink.muted,
        "--ink-faint": ink.faint,
        "--ink-border": ink.border,
        "--ink-divider": ink.divider,
        "--ink-accent": ink.accent,
        "--ink-error": ink.error,
        "--ink-warning": ink.warning,
        "--ink-success": ink.success,
        "--ink-info": ink.info,
        "--ink-focus": ink.focus,
        "--ink-hover": ink.hover,
        "--ink-selected": ink.selected,
    };
    for (const [token, value] of Object.entries(SURFACE_TOKEN_MAP)) vars[token] = value;
    for (const [token, source] of Object.entries(INK_TOKEN_MAP)) vars[token] = `var(${source})`;
    // `rgba(var(--accent-color-rgb), a)` is how the theme tints with the accent, and it needs the channels, not a colour.
    const triplet = rgbTriplet(ink.accent);
    if (triplet) vars["--accent-color-rgb"] = triplet;
    for (const [token, hex] of Object.entries(ansi ?? {})) vars[token] = readableOn(hex, surfaces);
    return vars;
}

// ── The registry: one signal per block ──────────────────────────────────────

const scopes = new Map<string, ReturnType<typeof createSignal<ScopeVars | undefined>>>();

function slot(blockId: string) {
    let s = scopes.get(blockId);
    if (!s) {
        s = createSignal<ScopeVars | undefined>(undefined);
        scopes.set(blockId, s);
    }
    return s;
}

/** Set (or, with `undefined`, clear) the properties a block's pane carries. Readers of `getPaneScope` update. */
export function registerPaneScope(blockId: string, vars: ScopeVars | undefined): void {
    slot(blockId)[1](vars);
}

/** A block's scope as a signal; `undefined` while it has none. Safe to read before it is registered. */
export function getPaneScope(blockId: string): Accessor<ScopeVars | undefined> {
    return slot(blockId)[0];
}

/** Forget a block that is gone, so the registry doesn't keep every block it has ever held. */
export function releasePaneScope(blockId: string): void {
    const s = scopes.get(blockId);
    if (!s) return;
    s[1](undefined);
    scopes.delete(blockId);
}
