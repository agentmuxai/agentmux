// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a pane's color comes from, and its color for each role. The one place
 * that reads a block's color sources; every surface (header, ring, pills,
 * Swarm rows, the activity flash) asks here.
 * docs/specs/SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.1–3.2.
 */

import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { paneRoleColor, type PaneColorRole } from "./pane-color-scheme";
import { getPaneTab, type PaneTabManifest } from "./pane-tab-registry";

/** `pick`: the pane's own "Pane Color". `agent`: the agent's identity color.
 *  `widget`: the color of the pane's widget type (Settings, else built-in). */
export type PaneHueSource = "pick" | "agent" | "widget";

export interface PaneIdentity {
    /** A hue on the Pane Color scale (`frame:hue`, widget colors). */
    hslHue?: number;
    /** The agent's identity color, `#rrggbb`. */
    hex?: string;
    source: PaneHueSource;
}

export interface PaneIdentityOptions {
    /** `false`: only the pane's own pick or agent color counts, as for the
     *  header tail's identity count (spec §3.8). Default `true`. */
    widget?: boolean;
}

/** The manifest whose color a view uses: its own, or for a legacy view the
 *  one it stands in for (`legacyOf`). */
function colorManifest(view: string | null | undefined): PaneTabManifest | undefined {
    const manifest = getPaneTab(view);
    return manifest?.legacyOf ? getPaneTab(manifest.legacyOf) : manifest;
}

/** The `pane:colors` key a view's color is stored under: its canonical view,
 *  after aliases and `legacyOf`. */
export function widgetColorView(view: string | null | undefined): string | undefined {
    return colorManifest(view)?.view ?? (view || undefined);
}

/** Every `pane:colors` key a view's color may be stored under: its canonical
 *  view first, then its former ids (its manifest's aliases). */
export function widgetColorKeys(view: string | null | undefined): string[] {
    const key = widgetColorView(view);
    return key ? [key, ...(colorManifest(view)?.aliases ?? [])] : [];
}

/** The user's color for a view's widget type: a hue, `null` for none, or
 *  undefined when unset. A color saved under a former id still counts. */
export function storedWidgetHue(view: string | null | undefined): number | null | undefined {
    const colors = getSettingsKeyAtom("pane:colors")();
    return widgetColorKeys(view)
        .map((name) => colors?.[name])
        .find((v) => v !== undefined);
}

/** The color of a widget type: the user's `pane:colors` entry, else the
 *  manifest's `defaultHue`. Undefined when it has none, or the user set `null`. */
export function widgetHueFor(view: string | null | undefined): number | undefined {
    if (!widgetColorView(view)) return undefined;
    const chosen = storedWidgetHue(view);
    if (chosen === null) return undefined;
    if (typeof chosen === "number") return chosen;
    return colorManifest(view)?.defaultHue;
}

/** The highest-precedence color source of a block: its own pick, then its
 *  agent's color, then its widget type's color. */
export function resolvePaneIdentity(
    meta: Block["meta"] | undefined,
    opts: PaneIdentityOptions = {}
): PaneIdentity | undefined {
    const hue = meta?.["frame:hue"];
    if (typeof hue === "number") return { hslHue: hue, source: "pick" };
    const hex = meta?.["frame:activebordercolor"];
    if (typeof hex === "string" && hex) return { hex, source: "agent" };
    if (opts.widget === false) return undefined;
    const widgetHue = widgetHueFor(meta?.view);
    return widgetHue === undefined ? undefined : { hslHue: widgetHue, source: "widget" };
}

/** A block's color for `role` in the current theme, or undefined when it has none. */
export function blockRoleColor(
    meta: Block["meta"] | undefined,
    isLightTheme: boolean,
    role: PaneColorRole,
    opts?: PaneIdentityOptions
): string | undefined {
    const id = resolvePaneIdentity(meta, opts);
    return id ? paneRoleColor(id.hslHue, id.hex, isLightTheme, role) : undefined;
}

/** A widget type's own color for `role` in the current theme (the top bar,
 *  the Launcher), or undefined when the type has none. */
export function widgetRoleColor(
    view: string | null | undefined,
    isLightTheme: boolean,
    role: PaneColorRole
): string | undefined {
    const hue = widgetHueFor(view);
    return hue === undefined ? undefined : paneRoleColor(hue, undefined, isLightTheme, role);
}

/** Whether the current theme is a light one. Reactive. */
export function isLightThemeActive(): boolean {
    const themeId = getSettingsKeyAtom("window:theme")();
    return typeof themeId === "string" && LIGHT_THEME_IDS.has(themeId);
}
