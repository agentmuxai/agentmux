// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shared identity palette: 16 hues for panes, agents, window tabs, widgets
 * and browser profiles, evenly spaced by human perception
 * (docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md).
 *
 * The list is `schema/color-palette.json`, read here and by srv
 * (crates/srv/src/backend/palette.rs), so there is one copy. A slot is a hue:
 * every surface derives its own tone from it per role and theme
 * (block/pane-color-scheme.ts), so sharing the palette never makes a tab
 * recolour a pane border. This module is the only place that knows the list;
 * ask `slotOf` instead of keeping an array of colours.
 */

import palette from "../../../schema/color-palette.json";
import { hslToRgb, oklchHueOfHex, oklchHueOfSrgb } from "./oklch";

export interface PaletteSlot {
    /** Lower-case name: what a pane's own pick and a widget's colour store. */
    id: string;
    name: string;
    /** OKLCH hue, degrees. */
    hue: number;
    /** The stored, theme-neutral `#rrggbb` (an agent, a window tab, a profile keep this). */
    hex: string;
}

/** The 16 slots, in hue order. */
export const PALETTE: ReadonlyArray<PaletteSlot> = palette.slots;

/** Circular distance between two hues, degrees, 0 to 180. */
export function hueDistance(a: number, b: number): number {
    return Math.abs((((a - b + 540) % 360) + 360) % 360 - 180);
}

export function slotById(id: string | null | undefined): PaletteSlot | undefined {
    const key = typeof id === "string" ? id.trim().toLowerCase() : "";
    return key ? PALETTE.find((s) => s.id === key) : undefined;
}

function nearestSlot(hue: number): PaletteSlot {
    let best = PALETTE[0];
    let bestDistance = Infinity;
    for (const s of PALETTE) {
        const d = hueDistance(hue, s.hue);
        if (d < bestDistance) {
            bestDistance = d;
            best = s;
        }
    }
    return best;
}

/**
 * The slot a stored colour stands for. Accepts a slot id, any `#rrggbb` (a
 * slot's own hex, an old Tailwind hex, an HSL pick's hex) or a legacy HSL hue
 * number (the old Pane Color menu and `pane:colors`, `hsl(h, 65%, 52%)`), and
 * snaps to the slot nearest in OKLCH hue. Anything else is no colour:
 * undefined.
 */
export function slotOf(value: string | number | null | undefined): PaletteSlot | undefined {
    if (typeof value === "number") {
        return Number.isFinite(value) ? nearestSlot(oklchHueOfSrgb(hslToRgb(((value % 360) + 360) % 360, 65, 52))) : undefined;
    }
    if (typeof value !== "string") return undefined;
    const byId = slotById(value);
    if (byId) return byId;
    const hue = oklchHueOfHex(value);
    return hue === undefined ? undefined : nearestSlot(hue);
}

/** FNV-1a (64-bit) of an id, the same hash srv uses (`palette.rs`). */
function fnv1a(id: string): bigint {
    const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
    const prime = BigInt("0x100000001b3");
    let hash = BigInt("0xcbf29ce484222325");
    for (let i = 0; i < id.length; i++) {
        hash ^= BigInt(id.charCodeAt(i) & 0xff);
        hash = (hash * prime) & mask;
    }
    return hash;
}

function usage(taken: Iterable<string>): number[] {
    const counts = PALETTE.map(() => 0);
    for (const value of taken) {
        const slot = slotOf(value);
        if (slot) counts[PALETTE.indexOf(slot)] += 1;
    }
    return counts;
}

/**
 * The slot for a new agent: the least-used one among `taken` (the colours the
 * existing agents keep), ties broken by walking forward from the id's hash
 * slot, so the same id and the same set always give the same colour. With
 * nothing taken it is the plain hash.
 */
export function pickSlot(agentId: string, taken: Iterable<string> = []): PaletteSlot {
    const counts = usage(taken);
    const min = Math.min(...counts);
    const start = Number(fnv1a(agentId) % BigInt(PALETTE.length));
    for (let k = 0; k < PALETTE.length; k++) {
        const i = (start + k) % PALETTE.length;
        if (counts[i] === min) return PALETTE[i];
    }
    return PALETTE[start];
}

/**
 * The slot for a new browser profile: the least-used one, then the one
 * farthest in hue from the slots already used, then the lowest.
 */
export function farthestFreeSlot(used: Iterable<string> = []): PaletteSlot {
    const counts = usage(used);
    const min = Math.min(...counts);
    const taken = PALETTE.filter((_, i) => counts[i] > 0);
    let best = PALETTE[0];
    let bestDistance = -1;
    PALETTE.forEach((s, i) => {
        if (counts[i] !== min) return;
        const d = taken.length === 0 ? 0 : Math.min(...taken.map((t) => hueDistance(s.hue, t.hue)));
        if (d > bestDistance + 1e-9) {
            bestDistance = d;
            best = s;
        }
    });
    return best;
}
