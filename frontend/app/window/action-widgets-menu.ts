// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared "Open in ..." actions for a leaf widget's right-click menu.
 *
 * Consumed by BOTH of action-widgets.tsx's menu-rendering paths — the native
 * OS context menu (pinned-bar icon right-click, via ContextMenuModel) and the
 * DOM PopoverMenu (More-dropdown / pinned-parent-flyout item right-click) —
 * so the two menus read from one source of truth instead of each hand-building
 * the same three items and silently drifting apart. See
 * docs/specs/SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_PHASE_2_2026_09_16.md.
 */

import { openViewInNewTab } from "@/app/tab/tab-presets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";

export interface WidgetOpenAction {
    label: string;
    run: () => void;
}

/**
 * Returns the three "Open in ..." actions for the given leaf widget, or an
 * empty array if `shortName` doesn't resolve to a widget with a view (e.g. a
 * parent/group widget — callers already special-case those before reaching
 * here, see action-widgets.tsx's `isParent` checks).
 */
export function buildWidgetOpenActions(
    shortName: string,
    wmap: Record<string, WidgetConfigType>
): WidgetOpenAction[] {
    const widgetDef = wmap[`defwidget@${shortName}`];
    const blockMeta = widgetDef?.blockdef?.meta as Record<string, unknown> | undefined;
    const view = (blockMeta?.["view"] as string) ?? null;
    if (!view) return [];
    return [
        {
            label: "Open in New Window",
            run: () => fireAndForget(async () => getApi().openNewWindowWithView(view, blockMeta)),
        },
        {
            label: "Open in Floating Pane",
            run: () =>
                fireAndForget(async () =>
                    TabRpcClient.rpcCall("pane.open", { view, meta: blockMeta, floating: true }, {})
                ),
        },
        {
            label: "Open in New Tab",
            run: () => fireAndForget(async () => openViewInNewTab(view, blockMeta)),
        },
    ];
}
