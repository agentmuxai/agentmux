// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which panes a click brings back instead of opening another (SPEC_WIDGET_SINGLETON_FOCUS_OR_NEW):
 * the `singleton` flag of the widget entry that opens the view (built-ins and widgets.json), or of
 * an installed widget's pane. The widget bar, the More menu, the palette and an agent's open all
 * ask here.
 */

import { getPaneTab } from "@/app/block/pane-tab-registry";
import { atoms } from "@/app/store/global";

/**
 * The built-in management panes, as `widgets.json` flags them. A user's `widgets.json` that replaces
 * a built-in entry drops the flag with the rest of the entry, so the policy of the view itself is
 * the fallback: such an entry keeps doing what it did (a test keeps this list equal to the flags).
 */
export const BUILTIN_SINGLETON_VIEWS: ReadonlySet<string> = new Set([
    "connectors",
    "drone",
    "help",
    "kanban",
    "memory",
    "settings",
    "swarm",
    "toolchain",
    "tower",
    "warden",
]);

/** Whether opening `view` brings back the pane already open: a widget entry for it says so, it is a built-in management pane, or its extension pane says so. */
export function isSingletonView(view: string | undefined): boolean {
    if (!view) return false;
    if (BUILTIN_SINGLETON_VIEWS.has(view)) return true;
    const widgets = atoms.fullConfigAtom()?.widgets ?? {};
    for (const widget of Object.values(widgets)) {
        const v = (widget.blockdef?.meta as Record<string, unknown> | undefined)?.["view"];
        if (v === view && widget.singleton === true) return true;
    }
    return getPaneTab(view)?.singleton === true;
}
