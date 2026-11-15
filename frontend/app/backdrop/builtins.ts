// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The 16 built-in backdrops (spec §3.2): the 16 hues of the shared pane palette, each as a deep tone for a dark app
 * theme and a pale tone for a light one, so one name works under both. A built-in is a seed; its ink comes from
 * `deriveInk`, never from a table (the override table below is empty until a derived result is disliked, and each
 * entry then needs a reason).
 */

import { oklchToHex } from "../theme/oklch";
import { PALETTE, type PaletteSlot } from "../theme/palette";
import { deriveInk, type DerivedBackdrop, type Signals } from "./ink";

export type AppScheme = "dark" | "light";

/** OKLCH lightness and chroma of the surface. The deep tone keeps clear of the dead zone (spec §3.5), as does the pale. */
export const BUILTIN_TONES: Record<AppScheme, { l: number; c: number }> = {
    dark: { l: 0.23, c: 0.05 },
    light: { l: 0.955, c: 0.025 },
};

/** A hand override: the surface a backdrop uses instead of the derived tone, and why. */
export const BUILTIN_OVERRIDES: Partial<Record<`${string}:${AppScheme}`, { surface: string; reason: string }>> = {};

export interface Builtin {
    /** `crimson`, `teal`, ...: the palette slot's id, which is also what the backdrop stores. */
    id: string;
    name: string;
    hue: number;
}

export const BUILTINS: ReadonlyArray<Builtin> = PALETTE.map((s: PaletteSlot) => ({ id: s.id, name: s.name, hue: s.hue }));

export function builtinSurface(id: string, scheme: AppScheme): string | undefined {
    const slot = PALETTE.find((s) => s.id === id);
    if (!slot) return undefined;
    const override = BUILTIN_OVERRIDES[`${id}:${scheme}`];
    if (override) return override.surface;
    const { l, c } = BUILTIN_TONES[scheme];
    return oklchToHex(l, c, slot.hue);
}

/** The backdrop for a built-in under an app theme, or undefined for an id that is not one. */
export function deriveBuiltin(id: string, scheme: AppScheme, signals?: Signals): DerivedBackdrop | undefined {
    const surface = builtinSurface(id, scheme);
    return surface ? deriveInk(surface, signals) : undefined;
}
