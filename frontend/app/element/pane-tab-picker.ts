// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "+" action shared by EVERY pane type — opens the same native widget
 * picker the widget bar's own right-click menu builds, and adds whatever is
 * picked to THIS pane's stack as a new Pane Tab.
 *
 * Extracted so agent and term use the identical action the nine
 * PaneChrome-driven widget types already did, rather than each
 * keeping its own single-purpose handler ("+ forks another agent" /
 * "+ opens another shell"). Those handlers made agent/term the only pane
 * types you could NOT add a different widget type to — you were stuck with
 * more of the same. Repo-owner directive: all panes are generic, there is
 * no such thing as a non-generic pane.
 *
 * Note this does not remove the old capability: picking "Agent" from the
 * menu creates an agent tab in this pane, as `handleNewAgentTab` did (and
 * "Terminal" likewise for `handleTermTabAdd`) — now via
 * `pane.open { stack_onto_block_id }`, which creates and places the block in
 * one backend step (`addWidgetAsPaneTab`). It just costs one menu step, and
 * every other widget type is reachable the same way.
 */

import { ContextMenuModel } from "@/app/store/contextmenu";
import { atoms, pushNotification } from "@/app/store/global";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { addWidgetAsPaneTab } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { effectiveStack } from "@/layout/lib/layoutStack";
import { findNode } from "@/layout/lib/layoutNode";
import { docTabHost, type DocTabHost } from "@/app/doc-tabs/doc-tab-hosts";

/** The Editor already in pane `nodeId`'s stack, if one is mounted. */
function editorHostOfPane(model: LayoutModel, nodeId: string): DocTabHost | undefined {
    const data = findNode(model.treeState.rootNode, nodeId)?.data;
    if (!data) return undefined;
    return effectiveStack(data)
        .map((id) => docTabHost(id))
        .find((h) => h?.docType === "editor" && (h.scope?.() ?? "") === "" && h.newDocument);
}

export function openPaneTabWidgetPicker(
    model: LayoutModel,
    nodeId: string,
    e: MouseEvent,
    /** The destination pane's own `PaneChromeModel.newTabMeta`, if it has
     *  one — see that field's doc comment. Passed in rather than read here
     *  so this stays a pure "open picker, add what's picked" helper with no
     *  knowledge of any view type. */
    newTabMeta?: (view: string | undefined) => Record<string, unknown> | undefined
): void {
    const onSelect = (blockDef: BlockDef) => {
        // Surfaced rather than swallowed — `handleNewAgentTab` (the agent
        // pane's own former "+") already did this for a failed `pane.open`,
        // and folding it in here means the eight other pane types that
        // previously failed silently get the same feedback.
        const view = (blockDef.meta as Record<string, unknown> | undefined)?.["view"] as string | undefined;
        // An Editor picked in a pane that has one is a new document tab of it, not a second Editor
        // pane tab (SPEC_DOCUMENT_PANES_MERGE_DOCUMENT_TABS_2026_10_30.md §1, row 3).
        const here = view === "editor" ? editorHostOfPane(model, nodeId) : undefined;
        if (here?.newDocument) {
            here.newDocument();
            return;
        }
        void addWidgetAsPaneTab(model, nodeId, blockDef, newTabMeta?.(view)).catch((err: unknown) => {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "New tab failed",
                message: err instanceof Error ? err.message : String(err),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
        });
    };
    // Live: `build` reads the widget config, so the open picker follows a widget being hidden,
    // shown or added (SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md).
    const build = () =>
        buildPaneWidgetMenuItems(atoms.fullConfigAtom()?.widgets ?? {}, atoms.fullConfigAtom()?.settings ?? {}, onSelect);
    ContextMenuModel.showLiveContextMenu(build, e);
}
