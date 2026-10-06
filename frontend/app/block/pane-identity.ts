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
import { getPaneTab } from "./pane-tab-registry";

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

/** The color of a widget type: the user's `pane:colors` entry, else the
 *  manifest's `defaultHue`. Undefined when it has none, or the user set `null`. */
export function widgetHueFor(view: string | null | undefined): number | undefined {
    const manifest = getPaneTab(view);
    const key = manifest?.view ?? view;
    if (!key) return undefined;
    const chosen = getSettingsKeyAtom("pane:colors")()?.[key];
    if (chosen === null) return undefined;
    if (typeof chosen === "number") return chosen;
    return manifest?.defaultHue;
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

/** Whether the current theme is a light one. Reactive. */
export function isLightThemeActive(): boolean {
    const themeId = getSettingsKeyAtom("window:theme")();
    return typeof themeId === "string" && LIGHT_THEME_IDS.has(themeId);
}
