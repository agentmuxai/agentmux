// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "+" action shared by EVERY pane type — opens the same native widget
 * picker the widget bar's own right-click menu builds, and adds whatever is
 * picked to THIS pane's stack as a new Pane Tab.
 *
 * Extracted so agent and term use the identical action the nine
 * GenericPaneChrome-driven widget types already did, rather than each
 * keeping its own single-purpose handler ("+ forks another agent" /
 * "+ opens another shell"). Those handlers made agent/term the only pane
 * types you could NOT add a different widget type to — you were stuck with
 * more of the same. Repo-owner directive: all panes are generic, there is
 * no such thing as a non-generic pane.
 *
 * Note this does not remove the old capability: picking "Agent" from the
 * menu runs the same `pane.open { view: "agent", skip_placement: true }` +
 * push that `handleNewAgentTab` did (and "Terminal" likewise for
 * `handleTermTabAdd`) — it just costs one menu step, and every other widget
 * type is now reachable the same way.
 */

import { ContextMenuModel } from "@/app/store/contextmenu";
import { atoms, pushNotification } from "@/app/store/global";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { addWidgetAsPaneTab } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";

export function openPaneTabWidgetPicker(model: LayoutModel, nodeId: string, e: MouseEvent): void {
    const wmap = atoms.fullConfigAtom()?.widgets ?? {};
    const settings = atoms.fullConfigAtom()?.settings ?? {};
    const items = buildPaneWidgetMenuItems(wmap, settings, (blockDef) => {
        // Surfaced rather than swallowed — `handleNewAgentTab` (the agent
        // pane's own former "+") already did this for a failed `pane.open`,
        // and folding it in here means the eight other pane types that
        // previously failed silently get the same feedback.
        void addWidgetAsPaneTab(model, nodeId, blockDef).catch((err: unknown) => {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "New tab failed",
                message: err instanceof Error ? err.message : String(err),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
        });
    });
    ContextMenuModel.showContextMenu(items, e);
}
