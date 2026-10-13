// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure config helpers for the widget bar — pinned/more widget derivation and
 * pin/unpin RPC persistence. Extracted from action-widgets.tsx.
 *
 * Settings keys:
 *   widget:pinned   — ordered short-name array (e.g. ["agent","terminal","sysinfo"])
 *   widget:hidden   — full keys hidden from the bar, More and the pane pickers
 *                     (SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md)
 *   widget:icononly — icons only, no labels
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createBlock, openOrFocusPaneByView } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { isLightThemeActive, widgetRoleColor } from "@/app/block/pane-identity";
import { paneRoleColor, type PaneColorRole } from "@/app/block/pane-color-scheme";
import type { JSX } from "solid-js";

/**
 * Short-names (no "defwidget@" prefix) that appear in some widget's
 * `children` list — i.e. every grouped child across all parent widgets,
 * purely structural (independent of pin state; see
 * getEffectiveGroupedChildKeys for the pin-state-aware version most
 * callers actually want). A parent itself is never in this set unless
 * groups are nested (out of scope — see
 * SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md §2).
 */
export function getGroupedChildKeys(wmap: Record<string, WidgetConfigType>): Set<string> {
    const grouped = new Set<string>();
    for (const w of Object.values(wmap ?? {})) {
        for (const child of w.children ?? []) grouped.add(child);
    }
    return grouped;
}

/** A widget entry's color for `role`: its own `color` (a messenger's brand,
 *  though it opens a browser), else the color of the view it opens
 *  (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.6). An entry color the
 *  palette can't read (any CSS color but `#rrggbb`) is used as written at
 *  full strength, and gives way to the view's color for the other roles. */
export function widgetEntryColor(widget: WidgetConfigType, role: PaneColorRole): string | undefined {
    const isLight = isLightThemeActive();
    if (widget.color) {
        const color = paneRoleColor(undefined, widget.color, isLight, role);
        if (color) return color;
        if (role === "identity") return widget.color;
    }
    return widgetRoleColor(widget.blockdef?.meta?.view, isLight, role);
}

/** A top-bar widget icon's style: the entry's color, faintly. */
export function widgetIconStyle(widget: WidgetConfigType): JSX.CSSProperties {
    const tint = widgetEntryColor(widget, "widgetTint");
    return tint ? { "--widget-tint": tint } : {};
}

// ── Hidden entries (widget:hidden) ─────────────────────────────────────────────

/** Entries that can't be hidden: Settings is the way back to everything else that was. */
export const UNHIDEABLE_WIDGET_KEYS: ReadonlySet<string> = new Set(["defwidget@settings"]);

const DEFWIDGET = "defwidget@";

/** The full keys hidden from the widget bar, More and the pane "+" pickers (`widget:hidden`).
 *  An entry that can't be hidden is never in the set. */
export function getHiddenKeys(settings: Record<string, any>): Set<string> {
    const raw = settings?.["widget:hidden"];
    const hidden = new Set<string>();
    if (!Array.isArray(raw)) return hidden;
    for (const key of raw) {
        if (typeof key === "string") hidden.add(key);
    }
    for (const key of UNHIDEABLE_WIDGET_KEYS) hidden.delete(key);
    return hidden;
}

/** True if the entry is hidden: listed in `widget:hidden`, or a group whose every child is
 *  hidden (an empty flyout reads as broken, so the parent goes too). */
export function isWidgetHidden(
    key: string,
    wmap: Record<string, WidgetConfigType>,
    settings: Record<string, any>,
    hidden: Set<string> = getHiddenKeys(settings)
): boolean {
    if (hidden.has(key)) return true;
    const children = wmap[key]?.children ?? [];
    if (children.length === 0) return false;
    const remaining = children.filter((name) => !isIndividuallyPinned(name, settings));
    return remaining.length > 0 && remaining.every((name) => hidden.has(DEFWIDGET + name));
}

/** True if `shortName` is individually pinned via widget:pinned — i.e. promoted out of its parent group. */
function isIndividuallyPinned(shortName: string, settings: Record<string, any>): boolean {
    const pinned: string[] | undefined = settings?.["widget:pinned"];
    return pinned !== undefined && pinned.includes(shortName);
}

/**
 * Grouped short-names that should stay hidden from every flat top-level
 * enumeration (pinned bar, More, Pin Widgets, the pane-widget pickers, the Launcher
 * grid, ...) — i.e. structurally a child (getGroupedChildKeys) AND NOT
 * individually pinned.
 *
 * A user can right-click a child inside its parent's flyout/submenu and
 * "Pin to bar" it — that PROMOTES it out of the group: it becomes a normal
 * standalone widget everywhere (shows on the bar/in More/checked in Pin
 * Widgets, like any other pinned widget) and drops out of its parent's own
 * children list, so it's never shown in two places at once. "Unpin from
 * bar" reverses this — the child goes back to being reachable only through
 * its parent, regardless of whether the parent itself is pinned.
 */
export function getEffectiveGroupedChildKeys(
    wmap: Record<string, WidgetConfigType>,
    settings: Record<string, any>
): Set<string> {
    const grouped = getGroupedChildKeys(wmap);
    const effective = new Set<string>();
    for (const shortName of grouped) {
        if (!isIndividuallyPinned(shortName, settings)) effective.add(shortName);
    }
    return effective;
}

/**
 * Resolve a parent widget's children to their full widget defs, in the
 * order declared by `children`. Skips any short-name that doesn't resolve
 * (e.g. a stale/typo'd entry in widgets.json).
 *
 * When `settings` is passed, a child that's currently individually pinned
 * (promoted out of the group — see getEffectiveGroupedChildKeys) is
 * excluded here too, so it doesn't render both as its own standalone entry
 * AND inside its old parent's flyout/submenu at the same time.
 */
export function getChildWidgets(
    widget: WidgetConfigType,
    wmap: Record<string, WidgetConfigType>,
    settings?: Record<string, any>
): { key: string; widget: WidgetConfigType }[] {
    const hidden = settings ? getHiddenKeys(settings) : new Set<string>();
    return (widget.children ?? [])
        .filter((shortName) => !settings || !isIndividuallyPinned(shortName, settings))
        .filter((shortName) => !hidden.has(DEFWIDGET + shortName))
        .map((shortName) => {
            const key = `defwidget@${shortName}`;
            return { key, widget: wmap[key] };
        })
        .filter((e): e is { key: string; widget: WidgetConfigType } => e.widget != null);
}

/**
 * Return the effective pinned short-names (no "defwidget@" prefix), in order.
 *
 * Priority:
 *  1. widget:pinned is set → authoritative. A grouped child listed here IS
 *     honored (not stripped) — that's exactly how an individual child gets
 *     promoted out of its parent group onto the bar (see
 *     getEffectiveGroupedChildKeys).
 *  2. Not set → derive from display:pinned in widget config. Grouped
 *     children are excluded here since none is ever authored
 *     display:pinned:true — this is about fresh-install defaults, not a
 *     user's deliberate promotion, so the group-hiding default still
 *     applies.
 */
export function getPinnedKeys(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): string[] {
    const hidden = getHiddenKeys(settings);
    return getSavedPinNames(settings, wmap).filter((shortName) => !isWidgetHidden(`defwidget@${shortName}`, wmap, settings, hidden));
}

/**
 * The pins as saved, hidden or not: `widget:pinned` (or the fresh-install
 * default). Every write of `widget:pinned` starts from this, not from getPinnedKeys, so a pinned
 * group hidden only because its children are (isWidgetHidden) keeps its pin through other edits
 * and is back on the bar once a child is shown again.
 */
export function getSavedPinNames(settings: Record<string, any>, wmap: Record<string, WidgetConfigType>): string[] {
    const pinned: string[] | undefined = settings["widget:pinned"];
    if (pinned !== undefined) {
        return [...new Set(pinned)].filter((shortName) => wmap[`defwidget@${shortName}`] != null);
    }
    const grouped = getGroupedChildKeys(wmap);
    return Object.entries(wmap)
        .filter(([key, w]) => w["display:pinned"] && !grouped.has(key.replace("defwidget@", "")))
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key]) => key.replace("defwidget@", ""));
}

/** Put the visible pins in `visibleOrder` into the slots they hold in `saved`, leaving any pin
 *  that is hidden right now where it was. */
export function withVisibleOrder(saved: string[], visibleOrder: string[]): string[] {
    const visible = new Set(visibleOrder);
    const next = [...saved];
    let i = 0;
    for (let slot = 0; slot < next.length; slot++) {
        if (visible.has(next[slot])) next[slot] = visibleOrder[i++];
    }
    return next;
}

export function getPinnedWidgets(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): { key: string; widget: WidgetConfigType }[] {
    if (!wmap) return [];
    return getPinnedKeys(settings, wmap)
        .map((shortName) => {
            const key = `defwidget@${shortName}`;
            return { key, widget: wmap[key] };
        })
        .filter((e) => e.widget != null);
}

export function getMoreWidgets(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): { key: string; widget: WidgetConfigType }[] {
    if (!wmap) return [];
    const pinnedSet = new Set(getPinnedKeys(settings, wmap));
    const grouped = getGroupedChildKeys(wmap);
    const hidden = getHiddenKeys(settings);
    return Object.entries(wmap)
        .filter(([key]) => {
            const shortName = key.replace("defwidget@", "");
            return !pinnedSet.has(shortName) && !grouped.has(shortName) && !isWidgetHidden(key, wmap, settings, hidden);
        })
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key, widget]) => ({ key, widget }));
}

// ── Pane-widget pickers (empty-tab menu, pane-tab picker) ──────────────────────

/** Non-pane widget views excluded from any pane-widget picker (empty-tab menu, pane-tab picker). */
const NON_PANE_WIDGET_VIEWS = new Set(["devtools"]);

function paneWidgetSortKey(w: WidgetConfigType): [number, string] {
    return [w["display:order"] ?? 0, w.label ?? ""];
}

function comparePaneWidgets(a: WidgetConfigType, b: WidgetConfigType): number {
    const [orderA, labelA] = paneWidgetSortKey(a);
    const [orderB, labelB] = paneWidgetSortKey(b);
    if (orderA !== orderB) return orderA - orderB;
    return labelA.localeCompare(labelB);
}

/**
 * Build the menu items for a "pick a pane widget to open here" surface —
 * shared by the pane-tab picker (`pane-tab-picker.ts`) and the empty-tab
 * right-click menu (`tabcontent.tsx`).
 * They used to hand-roll near-identical flat `Object.values(wmap)` logic
 * that (pre SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md) never knew about
 * grouped children, so "Discord"/"Slack"/etc. showed up individually here
 * even once hidden from the widget bar. A child NOT individually pinned is
 * excluded as a flat entry and instead reachable through a native nested
 * submenu under its parent's own label (`type: "submenu"`, same OS-native
 * submenu support `ContextMenuModel.showContextMenu()` already renders). A
 * child that IS individually pinned (promoted out of its group — see
 * getEffectiveGroupedChildKeys) shows as a normal flat entry instead, and
 * is dropped from its old parent's submenu so it's never listed twice.
 */
export function buildPaneWidgetMenuItems(
    wmap: Record<string, WidgetConfigType>,
    settings: Record<string, any>,
    onSelect: (blockdef: BlockDef) => void
): ContextMenuItem[] {
    const toLeafItem = (widget: WidgetConfigType): ContextMenuItem | null => {
        const view = widget.blockdef?.meta?.["view"] as string | undefined;
        if (!view || NON_PANE_WIDGET_VIEWS.has(view)) return null;
        return { label: widget.label ?? "Unnamed", click: () => onSelect(widget.blockdef) };
    };

    const grouped = getEffectiveGroupedChildKeys(wmap, settings);
    const hidden = getHiddenKeys(settings);
    const topLevel = Object.entries(wmap)
        .filter(([key]) => !grouped.has(key.replace("defwidget@", "")))
        // Hidden in Settings → Widgets means gone from "+" too (owner, 2026-10-12).
        .filter(([key]) => !isWidgetHidden(key, wmap, settings, hidden))
        .map(([, widget]) => widget)
        .sort(comparePaneWidgets);

    const items: ContextMenuItem[] = [];
    for (const widget of topLevel) {
        if ((widget.children?.length ?? 0) > 0) {
            const submenu = getChildWidgets(widget, wmap, settings)
                .map(({ widget: child }) => child)
                .sort(comparePaneWidgets)
                .map(toLeafItem)
                .filter((item): item is ContextMenuItem => item != null);
            if (submenu.length > 0) {
                items.push({ label: widget.label ?? "Unnamed", type: "submenu", submenu });
            }
            continue;
        }
        const item = toLeafItem(widget);
        if (item) items.push(item);
    }
    return items;
}

// ── Widget actions ────────────────────────────────────────────────────────────

/** Views whose widget focuses the pane already open in this tab instead of
 *  opening another: there's one Connectors and one Memory to manage. */
const FOCUS_EXISTING_VIEWS = new Set(["connectors", "memory"]);

export async function handleWidgetSelect(widget: WidgetConfigType) {
    const view = widget.blockdef?.meta?.["view"] as string | undefined;
    if (view && FOCUS_EXISTING_VIEWS.has(view) && !widget.magnified) {
        await openOrFocusPaneByView(view, widget.blockdef);
        return;
    }
    createBlock(widget.blockdef, widget.magnified);
}

export function pinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getSavedPinNames(settings, wmap);
        if (current.includes(shortName)) return;
        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": [...current, shortName] });
    });
}

export function unpinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getSavedPinNames(settings, wmap);
        await RpcApi.SetConfigCommand(TabRpcClient, {
            "widget:pinned": current.filter((k) => k !== shortName),
        });
    });
}

// ── The bar model (Settings → Widgets → Widget bar) ────────────────────────────

/** Where an entry shows: pinned on the bar, in More, hidden from both, or (a group's
 *  child) inside its group's flyout. */
export type WidgetBarState = "pinned" | "more" | "hidden" | "group";

export interface WidgetBarEntry {
    key: string;
    /** `agent` for `defwidget@agent`; the full key for anything else. */
    shortName: string;
    widget: WidgetConfigType;
    state: WidgetBarState;
    /** False for Settings, the way back to everything hidden. */
    canHide: boolean;
    /** Only built-in (`defwidget@`) entries can be pinned today. */
    canPin: boolean;
    /** A group's children, each with its own state ("group", "pinned" or "hidden"). */
    children: WidgetBarEntry[];
    /** For a pinned entry (a promoted child too): its place on the bar, and how many are pinned.
     *  Reorder limits come from here, not from the top-level rows, since a promoted child sits on
     *  the bar but is listed under its group. */
    pinIndex?: number;
    pinCount?: number;
}

function byOrder(a: WidgetConfigType, b: WidgetConfigType): number {
    return comparePaneWidgets(a, b);
}

/**
 * Every entry the widget bar knows about, with where it shows, in bar order: pinned first in
 * pin order, then More by `display:order`, then hidden. A group's children are nested under
 * it. Settings → Widgets renders this; the bar's own lists (getPinnedWidgets,
 * getMoreWidgets, getChildWidgets) apply the same rules, so the two can't disagree.
 */
export function getWidgetBarModel(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): WidgetBarEntry[] {
    if (!wmap) return [];
    const hidden = getHiddenKeys(settings);
    const pinned = getPinnedKeys(settings, wmap);
    const pinnedSet = new Set(pinned);
    const structuralChildren = getGroupedChildKeys(wmap);
    const entry = (key: string, widget: WidgetConfigType, state: WidgetBarState, children: WidgetBarEntry[] = []): WidgetBarEntry => {
        const shortName = key.startsWith(DEFWIDGET) ? key.slice(DEFWIDGET.length) : key;
        return {
            key,
            shortName,
            widget,
            state,
            canHide: !UNHIDEABLE_WIDGET_KEYS.has(key),
            canPin: key.startsWith(DEFWIDGET),
            children,
            ...(state === "pinned" ? { pinIndex: pinned.indexOf(shortName), pinCount: pinned.length } : {}),
        };
    };
    const childrenOf = (widget: WidgetConfigType): WidgetBarEntry[] =>
        (widget.children ?? [])
            .map((name) => [DEFWIDGET + name, wmap[DEFWIDGET + name]] as const)
            .filter((pair): pair is readonly [string, WidgetConfigType] => pair[1] != null)
            .map(([key, child]) =>
                entry(key, child, hidden.has(key) ? "hidden" : pinnedSet.has(key.slice(DEFWIDGET.length)) ? "pinned" : "group")
            );

    const topLevel = Object.entries(wmap).filter(([key]) => !structuralChildren.has(key.replace(DEFWIDGET, "")));
    const pinnedRows = pinned
        .map((name) => DEFWIDGET + name)
        .filter((key) => !structuralChildren.has(key.slice(DEFWIDGET.length)) && wmap[key] != null)
        .map((key) => entry(key, wmap[key], "pinned", childrenOf(wmap[key])));
    const rest = topLevel
        .filter(([key]) => !pinnedSet.has(key.replace(DEFWIDGET, "")))
        .sort(([, a], [, b]) => byOrder(a, b));
    const moreRows = rest
        .filter(([key]) => !isWidgetHidden(key, wmap, settings, hidden))
        .map(([key, widget]) => entry(key, widget, "more", childrenOf(widget)));
    const hiddenRows = rest
        .filter(([key]) => isWidgetHidden(key, wmap, settings, hidden))
        .map(([key, widget]) => entry(key, widget, "hidden", childrenOf(widget)));
    return [...pinnedRows, ...moreRows, ...hiddenRows];
}

/** The keys `setWidgetBarState` would write; exported for tests. */
export function nextWidgetBarConfig(
    key: string,
    state: WidgetBarState,
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): { "widget:pinned"?: string[]; "widget:hidden"?: string[] | null } {
    const shortName = key.startsWith(DEFWIDGET) ? key.slice(DEFWIDGET.length) : null;
    const pinned = getSavedPinNames(settings, wmap);
    const rawHidden: string[] = Array.isArray(settings?.["widget:hidden"])
        ? settings["widget:hidden"].filter((k: unknown): k is string => typeof k === "string")
        : [];
    const withoutKey = rawHidden.filter((k) => k !== key);
    const out: { "widget:pinned"?: string[]; "widget:hidden"?: string[] | null } = {};
    if (state === "hidden") {
        if (UNHIDEABLE_WIDGET_KEYS.has(key)) return out;
        out["widget:hidden"] = [...withoutKey, key];
        if (shortName && pinned.includes(shortName)) out["widget:pinned"] = pinned.filter((n) => n !== shortName);
        return out;
    }
    // A group hidden only because every child is hidden (isWidgetHidden): showing or pinning the
    // group brings those children back into it, or the click would change nothing.
    let nextHidden = withoutKey;
    const children = wmap[key]?.children ?? [];
    if (children.length > 0 && isWidgetHidden(key, wmap, { ...settings, "widget:hidden": withoutKey })) {
        const childKeys = new Set(children.map((name) => DEFWIDGET + name));
        nextHidden = withoutKey.filter((k) => ![...getHiddenKeys({ "widget:hidden": [k] })].some((h) => childKeys.has(h)));
    }
    if (nextHidden.length !== rawHidden.length) out["widget:hidden"] = nextHidden.length > 0 ? nextHidden : null;
    if (state === "pinned") {
        if (shortName && !pinned.includes(shortName)) out["widget:pinned"] = [...pinned, shortName];
    } else if (shortName && pinned.includes(shortName)) {
        out["widget:pinned"] = pinned.filter((n) => n !== shortName);
    }
    return out;
}

/** Move an entry to pinned, More (or back into its group) or hidden, in one write, so the bar
 *  and Settings → Widgets update together from the config event that follows. */
export async function setWidgetBarState(
    key: string,
    state: WidgetBarState,
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): Promise<void> {
    const next = nextWidgetBarConfig(key, state, settings, wmap);
    if (Object.keys(next).length === 0) return;
    await RpcApi.SetConfigCommand(TabRpcClient, next as SettingsType);
}

/** Move a pinned entry one place left (-1) or right (+1). */
export async function movePinnedWidget(
    shortName: string,
    delta: -1 | 1,
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): Promise<void> {
    const visible = getPinnedKeys(settings, wmap);
    const from = visible.indexOf(shortName);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= visible.length) return;
    const next = [...visible];
    [next[from], next[to]] = [next[to], next[from]];
    await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": withVisibleOrder(getSavedPinNames(settings, wmap), next) });
}

/** Back to the fresh-install bar: the default pins, nothing hidden. */
export async function resetWidgetBar(): Promise<void> {
    await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": null, "widget:hidden": null } as unknown as SettingsType);
}
