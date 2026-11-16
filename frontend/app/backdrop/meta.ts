// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's backdrop as block meta, and what it resolves to (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md
 * §8 and §9). The record holds the seed, never the colours: `resolveBackdrop` derives them for the app theme the
 * pane is under, so a theme switch re-derives and nothing stale is stored.
 */

import { hexToRgb } from "../theme/oklch";
import { BUILTINS, deriveBuiltin, type AppScheme } from "./builtins";
import { deriveInk, type DerivedBackdrop } from "./ink";

/**
 * The pane views whose own colours follow a backdrop (spec §6, §14). Phase 1 is the agent pane. A terminal keeps its
 * default text and ANSI colours, so a backdrop on one is unreadable; the other panes come with phase 2. A view is added
 * here only when its text can be shown to follow the ink.
 */
const BACKDROP_VIEWS: ReadonlySet<string> = new Set(["agent"]);

/** Whether a pane of this view takes a backdrop: it is offered in its menu, and a record on any other view is ignored. */
export const supportsBackdrop = (view: unknown): boolean => typeof view === "string" && BACKDROP_VIEWS.has(view);

/** The shape the record is written in. A reader that finds a newer `version` ignores the record. */
export const BACKDROP_VERSION = 1;

export type BackdropKind = "builtin" | "color" | "image";
export type BackdropPolarity = "follow" | "dark" | "light";

export interface BackdropMeta {
    kind: BackdropKind;
    /** `builtin`: the palette slot id (`teal`). */
    id?: string;
    /** `color`: `#rrggbb`. */
    hex?: string;
    /** `image`: the stored image's id. Not resolved before phase 3. */
    imageId?: string;
    polarity: BackdropPolarity;
    version: number;
}

const BUILTIN_IDS: ReadonlySet<string> = new Set(BUILTINS.map((b) => b.id));
const POLARITIES: ReadonlySet<string> = new Set(["follow", "dark", "light"]);
const HEX6 = /^#[0-9a-f]{6}$/i;

/**
 * The record in a pane's meta, or undefined when there is none or it can't be read: a wrong shape, an id that is
 * not one of the 16, a colour that is not `#rrggbb`, or a `version` newer than this build. A bad record must never
 * break a pane, so it reads as no backdrop.
 */
export function parseBackdrop(raw: unknown): BackdropMeta | undefined {
    if (raw == null || typeof raw !== "object") return undefined;
    const r = raw as Record<string, unknown>;
    const version = r.version === undefined ? BACKDROP_VERSION : r.version;
    if (typeof version !== "number" || !Number.isInteger(version) || version < 1 || version > BACKDROP_VERSION) return undefined;
    const polarity = r.polarity === undefined ? "follow" : r.polarity;
    if (typeof polarity !== "string" || !POLARITIES.has(polarity)) return undefined;
    const base = { polarity: polarity as BackdropPolarity, version };
    if (r.kind === "builtin" && typeof r.id === "string" && BUILTIN_IDS.has(r.id)) return { ...base, kind: "builtin", id: r.id };
    if (r.kind === "color" && typeof r.hex === "string" && HEX6.test(r.hex)) return { ...base, kind: "color", hex: r.hex.toLowerCase() };
    if (r.kind === "image" && typeof r.imageId === "string" && r.imageId !== "") return { ...base, kind: "image", imageId: r.imageId };
    return undefined;
}

/** Whether two records are the same backdrop, field by field: `parseBackdrop` makes a new object each time, so identity says nothing. */
export function sameBackdrop(a: BackdropMeta | undefined, b: BackdropMeta | undefined): boolean {
    if (a === b) return true;
    if (!a || !b) return false;
    return a.kind === b.kind && a.id === b.id && a.hex === b.hex && a.imageId === b.imageId && a.polarity === b.polarity && a.version === b.version;
}

/** Whether two resolved backdrops are the same one: the same origin and the same colours, not the same object. */
export function sameResolved(a: ResolvedBackdrop | undefined, b: ResolvedBackdrop | undefined): boolean {
    if (a === b) return true;
    if (!a || !b) return false;
    return a.id === b.id && JSON.stringify(a.derived) === JSON.stringify(b.derived);
}

/** A backdrop ready to apply: where it came from, and the surfaces and ink for the pane. */
export interface ResolvedBackdrop {
    /** What the pane's `data-backdrop` says: `builtin:teal`, `color` or `legacy`. */
    id: string;
    derived: DerivedBackdrop;
}

/** `#rgb` or `#rrggbb` as `#rrggbb`; undefined for anything else (a gradient, a url, a named colour). */
function plainHex(value: unknown): string | undefined {
    if (typeof value !== "string") return undefined;
    const v = value.trim();
    const long = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(v);
    const hex = long ? `#${long[1]}${long[1]}${long[2]}${long[2]}${long[3]}${long[3]}` : v;
    return HEX6.test(hex) && hexToRgb(hex) ? hex.toLowerCase() : undefined;
}

/**
 * What a pane's meta resolves to under an app theme, or undefined when the pane has no backdrop to apply (it then
 * keeps the app's tokens exactly as before).
 *
 * - A `backdrop` record wins. A built-in follows the app theme under `follow` and keeps its own side under `dark` or
 *   `light`; a custom colour is derived from the colour itself (a `polarity` does not change a colour the user chose).
 * - With no record, a `bg` that is a plain colour becomes an in-memory `legacy` backdrop (§8): the old value is still
 *   painted as before, and its ink is derived by the same function. A `bg` that is a gradient or a url stays untouched
 *   until an image can be measured (phase 3).
 * - An `image` record is not resolved before phase 3.
 */
export function resolveBackdrop(meta: { backdrop?: unknown; bg?: unknown } | undefined, appScheme: AppScheme): ResolvedBackdrop | undefined {
    const record = parseBackdrop(meta?.backdrop);
    if (record) {
        if (record.kind === "builtin") {
            const scheme = record.polarity === "follow" ? appScheme : record.polarity;
            const derived = deriveBuiltin(record.id!, scheme);
            return derived && { id: `builtin:${record.id}`, derived };
        }
        if (record.kind === "color") return { id: "color", derived: deriveInk(record.hex!) };
        return undefined;
    }
    const legacy = plainHex(meta?.bg);
    return legacy ? { id: "legacy", derived: deriveInk(legacy) } : undefined;
}
