// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure config helpers for the widget bar — pinned/more widget derivation and
 * pin/unpin RPC persistence. Extracted from action-widgets.tsx.
 *
 * Settings keys:
 *   widget:pinned   — ordered short-name array (e.g. ["agent","terminal","sysinfo"])
 *   widget:icononly — icons only, no labels
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createBlock } from "@/store/global";
import { fireAndForget } from "@/util/util";

/**
 * Short-names (no "defwidget@" prefix) that appear in some widget's
 * `children` list — i.e. every grouped child across all parent widgets.
 * These must never appear as their own top-level row (pinned or in More);
 * they're only reachable by expanding their parent's submenu. A parent
 * itself is never in this set unless groups are nested (out of scope —
 * see SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md §2).
 */
export function getGroupedChildKeys(wmap: Record<string, WidgetConfigType>): Set<string> {
    const grouped = new Set<string>();
    for (const w of Object.values(wmap ?? {})) {
        for (const child of w.children ?? []) grouped.add(child);
    }
    return grouped;
}

/**
 * Resolve a parent widget's children to their full widget defs, in the
 * order declared by `children`. Skips any short-name that doesn't resolve
 * (e.g. a stale/typo'd entry in widgets.json).
 */
export function getChildWidgets(
    widget: WidgetConfigType,
    wmap: Record<string, WidgetConfigType>
): { key: string; widget: WidgetConfigType }[] {
    return (widget.children ?? [])
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
 *  1. widget:pinned is set → authoritative
 *  2. Not set → derive from display:pinned in widget config
 *
 * Grouped children (§ getGroupedChildKeys) are always excluded — they're
 * only reachable through their parent's submenu, never as a standalone
 * pinned entry.
 */
export function getPinnedKeys(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): string[] {
    const grouped = getGroupedChildKeys(wmap);
    const pinned: string[] | undefined = settings["widget:pinned"];
    if (pinned !== undefined) {
        return pinned.filter((shortName) => wmap[`defwidget@${shortName}`] != null && !grouped.has(shortName));
    }
    return Object.entries(wmap)
        .filter(([key, w]) => w["display:pinned"] && !grouped.has(key.replace("defwidget@", "")))
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key]) => key.replace("defwidget@", ""));
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
    return Object.entries(wmap)
        .filter(([key]) => {
            const shortName = key.replace("defwidget@", "");
            return !pinnedSet.has(shortName) && !grouped.has(shortName);
        })
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key, widget]) => ({ key, widget }));
}

// ── Pane-widget pickers (Replace With..., empty-tab menu) ──────────────────────

/** Non-pane widget views excluded from any pane-widget picker (Replace With, empty-tab menu). */
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
 * shared by the "Replace With..." pane context-menu submenu
 * (`pane-actions.ts`) and the empty-tab right-click menu (`tabcontent.tsx`).
 * Both used to hand-roll near-identical flat `Object.values(wmap)` logic
 * that (pre SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md) never knew about
 * grouped children, so "Discord"/"Slack"/etc. showed up individually here
 * even once hidden from the widget bar. Grouped children are now excluded
 * as flat entries and instead reachable through a native nested submenu
 * under their parent's own label (`type: "submenu"`, same OS-native submenu
 * support `ContextMenuModel.showContextMenu()` already renders) — so the
 * group stays reachable everywhere a leaf widget would have been, without
 * the top-level list showing every child individually.
 */
export function buildPaneWidgetMenuItems(
    wmap: Record<string, WidgetConfigType>,
    onSelect: (blockdef: BlockDef) => void,
    opts: { excludeView?: string } = {}
): ContextMenuItem[] {
    const toLeafItem = (widget: WidgetConfigType): ContextMenuItem | null => {
        const view = widget.blockdef?.meta?.["view"] as string | undefined;
        if (!view || NON_PANE_WIDGET_VIEWS.has(view)) return null;
        if (opts.excludeView && view === opts.excludeView) return null;
        return { label: widget.label ?? "Unnamed", click: () => onSelect(widget.blockdef) };
    };

    const grouped = getGroupedChildKeys(wmap);
    const topLevel = Object.entries(wmap)
        .filter(([key]) => !grouped.has(key.replace("defwidget@", "")))
        .map(([, widget]) => widget)
        .sort(comparePaneWidgets);

    const items: ContextMenuItem[] = [];
    for (const widget of topLevel) {
        if ((widget.children?.length ?? 0) > 0) {
            const submenu = getChildWidgets(widget, wmap)
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

export async function handleWidgetSelect(widget: WidgetConfigType) {
    createBlock(widget.blockdef, widget.magnified);
}

export function pinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getPinnedKeys(settings, wmap);
        if (current.includes(shortName)) return;
        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": [...current, shortName] } as any);
    });
}

export function unpinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getPinnedKeys(settings, wmap);
        await RpcApi.SetConfigCommand(TabRpcClient, {
            "widget:pinned": current.filter((k) => k !== shortName),
        } as any);
    });
}
