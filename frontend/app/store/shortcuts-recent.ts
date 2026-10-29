// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Shortcuts pane's last-used order (docs/specs/SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.2).
 *
 * `launcher:recent` in the user's settings holds the last widget keys the person opened,
 * newest first (the key of the widget entry: `defwidget@terminal`, `ext@acme.board/main`), so
 * the order follows them to every window. It is written where a pane is opened by the person
 * (`block-layout-actions.ts`), never for an agent's `Pane` open, which does not go through here.
 * A widget that was never opened has no entry and keeps its configured place.
 */

import { fullConfigAtom } from "./config-signals";
import { RpcApi } from "./rpc-api";
import { TabRpcClient } from "./rpc-util";

/** How many keys the list keeps. */
export const SHORTCUTS_RECENT_MAX = 12;

/** The views that are not a widget of their own to come back to: the Shortcuts pane itself. */
const UNTRACKED_VIEWS: ReadonlySet<string> = new Set(["shortcuts", "launcher"]);

/** `recent` with `key` first, no repeats, at most `SHORTCUTS_RECENT_MAX`. Pure. */
export function pushRecent(recent: readonly string[] | undefined, key: string): string[] {
    return [key, ...(recent ?? []).filter((k) => k !== key)].slice(0, SHORTCUTS_RECENT_MAX);
}

/**
 * `entries` (widget key and entry, in their configured order) with the keys in `recent` first, in
 * `recent`'s order, then the rest in the order they came. A key in `recent` that is not among the
 * entries (hidden, uninstalled) is skipped. Pure.
 */
export function orderByRecent<T>(entries: readonly (readonly [string, T])[], recent: readonly string[] | undefined): [string, T][] {
    const byKey = new Map(entries.map((e) => [e[0], e] as const));
    const first: [string, T][] = [];
    for (const key of recent ?? []) {
        const entry = byKey.get(key);
        if (entry) {
            first.push([entry[0], entry[1]]);
            byKey.delete(key);
        }
    }
    return [...first, ...entries.filter((e) => byKey.has(e[0])).map((e): [string, T] => [e[0], e[1]])];
}

/**
 * The widget entry a block of `blockDef` was opened from: the entry whose view is the block's,
 * and where several entries share a view (Warden's sections, say), the first whose own meta the
 * block's meta carries. Undefined for a block no entry stands for. Pure.
 */
export function widgetKeyForBlockDef(wmap: Record<string, WidgetConfigType> | undefined, blockDef: BlockDef | undefined): string | undefined {
    const meta = blockDef?.meta as Record<string, unknown> | undefined;
    const view = meta?.["view"];
    if (typeof view !== "string" || UNTRACKED_VIEWS.has(view)) return undefined;
    const sameView = Object.entries(wmap ?? {}).filter(
        ([, w]) => !w.children?.length && (w.blockdef?.meta as Record<string, unknown> | undefined)?.["view"] === view
    );
    if (sameView.length <= 1) return sameView[0]?.[0];
    const carried = sameView.find(([, w]) =>
        Object.entries((w.blockdef?.meta ?? {}) as Record<string, unknown>).every(([k, v]) => JSON.stringify(meta?.[k]) === JSON.stringify(v))
    );
    return (carried ?? sameView[0])[0];
}

/**
 * The person opened a pane of `blockDef`: it comes first in the Shortcuts pane from now on. Does
 * nothing, and never throws, for a block no widget entry stands for or one that is first already.
 */
export function recordBlockOpened(blockDef: BlockDef | undefined): void {
    try {
        const config = fullConfigAtom();
        const key = widgetKeyForBlockDef(config?.widgets, blockDef);
        if (!key) return;
        const current = config?.settings?.["launcher:recent"];
        if (current?.[0] === key) return;
        void RpcApi.SetConfigCommand(TabRpcClient, { "launcher:recent": pushRecent(current, key) }).catch(() => {});
    } catch {
        // A failure to remember the order must never fail opening a pane.
    }
}
