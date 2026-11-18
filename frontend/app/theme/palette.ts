// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shared identity palette for panes, agents, window tabs, widgets and
 * browser profiles: 12 hues evenly spaced by human perception, then Black and
 * White (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md).
 *
 * The list is `schema/color-palette.json`, read here and by srv
 * (crates/srv-core/src/backend/palette.rs), so there is one copy. A slot is a
 * hue or a neutral: every surface derives its own tone from it per role and
 * theme (block/pane-color-scheme.ts), so sharing the palette never makes a tab
 * recolour a pane border. This module is the only place that knows the list;
 * ask `slotOf` instead of keeping an array of colours.
 */

import palette from "../../../schema/color-palette.json";
import { hslToRgb, oklchHueOfSrgb, oklchOfHex } from "./oklch";

export interface PaletteSlot {
    /** Lower-case name: what a pane's own pick and a widget's colour store. */
    id: string;
    name: string;
    /** OKLCH hue, degrees. Black and White have none. */
    hue?: number;
    /** Black or White: no hue, and a lightness of its own per role (pane-color-scheme.ts). */
    neutral?: "black" | "white";
    /** The stored, theme-neutral `#rrggbb` (an agent, a window tab, a profile keep this). */
    hex: string;
}

/** A slot that is a hue: all but Black and White. */
export type HueSlot = PaletteSlot & { hue: number };

/** The 14 slots: the 12 hues in hue order, then Black and White. */
export const PALETTE: ReadonlyArray<PaletteSlot> = palette.slots as ReadonlyArray<PaletteSlot>;

/** The 12 hues, in hue order. What is handed out automatically, and what a colour with a hue snaps to. */
export const HUE_SLOTS: ReadonlyArray<HueSlot> = PALETTE.filter((s): s is HueSlot => typeof s.hue === "number");

/** The ids and stored hexes of the 16 slots before version 2, each with the id of the slot it now means. */
const RETIRED_IDS: Readonly<Record<string, string>> = palette.retired.ids;
const RETIRED_HEXES: Readonly<Record<string, string>> = palette.retired.hexes;

/** A stored colour with less chroma than this is a neutral, not a hue. No slot and no older palette hex is that grey. */
const NEUTRAL_MAX_CHROMA = 0.02;
/** A neutral below this OKLCH lightness is Black, the rest White. */
const NEUTRAL_SPLIT_LIGHTNESS = 0.5;

/** Circular distance between two hues, degrees, 0 to 180. */
export function hueDistance(a: number, b: number): number {
    return Math.abs((((a - b + 540) % 360) + 360) % 360 - 180);
}

const current = (id: string): PaletteSlot | undefined => PALETTE.find((s) => s.id === id);

/** The slot with this id. An id retired with the 16 (`crimson`, `azure`, ...) gives the slot it became. */
export function slotById(id: string | null | undefined): PaletteSlot | undefined {
    const key = typeof id === "string" ? id.trim().toLowerCase() : "";
    if (!key) return undefined;
    return current(key) ?? (Object.hasOwn(RETIRED_IDS, key) ? current(RETIRED_IDS[key]) : undefined);
}

function nearestHueSlot(hue: number): PaletteSlot {
    let best = HUE_SLOTS[0];
    let bestDistance = Infinity;
    for (const s of HUE_SLOTS) {
        const d = hueDistance(hue, s.hue);
        if (d < bestDistance) {
            bestDistance = d;
            best = s;
        }
    }
    return best;
}

/** The slot a `#rrggbb` (the `#` optional) stands for, or undefined when it is not one. */
function slotOfHex(value: string): PaletteSlot | undefined {
    const color = oklchOfHex(value);
    if (!color) return undefined;
    const hex = value.trim().toLowerCase();
    const key = hex.startsWith("#") ? hex : `#${hex}`;
    // One of the 16's own hexes: the table says where it went. Its hue alone does not (Indigo sits
    // midway between Blue and Violet).
    if (Object.hasOwn(RETIRED_HEXES, key)) return current(RETIRED_HEXES[key]);
    if (color.c < NEUTRAL_MAX_CHROMA) return current(color.l < NEUTRAL_SPLIT_LIGHTNESS ? "black" : "white");
    return nearestHueSlot(color.h);
}

/**
 * The slot a stored colour stands for. Accepts a slot id (a retired one too),
 * any `#rrggbb` (a slot's own hex, one of the 16's, an old Tailwind hex, an
 * HSL pick's hex) or a legacy HSL hue number (the old Pane Color menu and
 * `pane:colors`, `hsl(h, 65%, 52%)`). A colour with a hue snaps to the hue
 * slot nearest in OKLCH hue; a grey is Black or White. Anything else is no
 * colour: undefined.
 */
export function slotOf(value: string | number | null | undefined): PaletteSlot | undefined {
    if (typeof value === "number") {
        return Number.isFinite(value) ? nearestHueSlot(oklchHueOfSrgb(hslToRgb(((value % 360) + 360) % 360, 65, 52))) : undefined;
    }
    if (typeof value !== "string") return undefined;
    return slotById(value) ?? slotOfHex(value);
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

/** How many of `taken` use each hue. A Black or White one uses no hue, so it is not counted. */
function usage(taken: Iterable<string>): number[] {
    const counts = HUE_SLOTS.map(() => 0);
    for (const value of taken) {
        const i = HUE_SLOTS.indexOf(slotOf(value) as HueSlot);
        if (i >= 0) counts[i] += 1;
    }
    return counts;
}

/**
 * The slot for a new agent: the least-used hue among `taken` (the colours the
 * existing agents keep), ties broken by walking forward from the id's hash
 * slot, so the same id and the same set always give the same colour. With
 * nothing taken it is the plain hash. Never Black or White: a neutral is only
 * ever someone's own choice.
 */
export function pickSlot(agentId: string, taken: Iterable<string> = []): PaletteSlot {
    const counts = usage(taken);
    const min = Math.min(...counts);
    const n = HUE_SLOTS.length;
    const start = Number(fnv1a(agentId) % BigInt(n));
    for (let k = 0; k < n; k++) {
        const i = (start + k) % n;
        if (counts[i] === min) return HUE_SLOTS[i];
    }
    return HUE_SLOTS[start];
}

/**
 * The slot for a new browser profile: the least-used hue, then the one
 * farthest in hue from the hues already used, then the lowest. Never Black or
 * White.
 */
export function farthestFreeSlot(used: Iterable<string> = []): PaletteSlot {
    const counts = usage(used);
    const min = Math.min(...counts);
    const taken = HUE_SLOTS.filter((_, i) => counts[i] > 0);
    let best = HUE_SLOTS[0];
    let bestDistance = -1;
    HUE_SLOTS.forEach((s, i) => {
        if (counts[i] !== min) return;
        const d = taken.length === 0 ? 0 : Math.min(...taken.map((t) => hueDistance(s.hue, t.hue)));
        if (d > bestDistance + 1e-9) {
            bestDistance = d;
            best = s;
        }
    });
    return best;
}
