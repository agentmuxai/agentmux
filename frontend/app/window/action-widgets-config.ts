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
