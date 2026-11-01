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
import { IDENTITY_META_KEY, parseIdentity } from "@/app/util/browser-identity";

/**
 * The built-in management panes, as `widgets.json` flags them. A user's `widgets.json` that replaces
 * a built-in entry drops the flag with the rest of the entry, so the policy of the view itself is
 * the fallback: such an entry keeps doing what it did (a test keeps this list equal to the flags).
 */
export const BUILTIN_SINGLETON_VIEWS: ReadonlySet<string> = new Set([
    "audit",
    "connectors",
    "drone",
    "help",
    "history",
    "kanban",
    "memory",
    "settings",
    "swarm",
    "toolchain",
    "tower",
    "vault",
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

/** Where a click looks for the pane it would bring back: this window only, or every window of the app. */
export type PaneScope = "window" | "app";

/**
 * Views whose pane holds something exclusive, so a second copy in another window would be a conflict, not a
 * convenience: a live agent session and an Editor document. Everything else is per window by default
 * (docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §6).
 */
export const APP_SCOPE_VIEWS: ReadonlySet<string> = new Set(["agent", "editor"]);

const asScope = (v: unknown): PaneScope | undefined => (v === "window" || v === "app" ? v : undefined);

/**
 * The scope of `view`: what the widget entry that opens it says (`scope`), else its extension pane, else
 * `app` for the exclusive views and for a browser tab on a named profile (`meta` is the pane's), else `window`.
 */
export function paneScope(view: string | undefined, meta?: Record<string, unknown>): PaneScope {
    if (!view) return "window";
    const widgets = atoms.fullConfigAtom()?.widgets ?? {};
    for (const widget of Object.values(widgets)) {
        const v = (widget.blockdef?.meta as Record<string, unknown> | undefined)?.["view"];
        const said = v === view ? asScope(widget.scope) : undefined;
        if (said) return said;
    }
    const registered = asScope(getPaneTab(view)?.scope);
    if (registered) return registered;
    if (APP_SCOPE_VIEWS.has(view)) return "app";
    if (view === "browser" && parseIdentity(meta?.[IDENTITY_META_KEY]).kind === "profile") return "app";
    return "window";
}
