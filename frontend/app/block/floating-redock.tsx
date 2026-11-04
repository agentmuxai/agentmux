// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Redock" for floating panes: put the pane back into a window's tile layout
// from a header button, with no drag. The only way back on native Wayland, where
// a floater's header drag belongs to the compositor and no window positions are
// known; elsewhere an explicit alternative to dragging onto a tile.
// docs/specs/SPEC_FLOATING_PANE_REDOCK_BUTTON_2026_10_15.md

import { PaneRestoreGlyph } from "@/app/element/pane-control-glyphs";
import * as MOS from "@/app/store/mos";
import { holdRedock } from "@/app/workspace/floating-redock-guard";
import { floaterPanes, redockFloaterPane } from "@/app/workspace/floater-panes";
import { atoms, getApi } from "@/store/global";
import { DeclIconButton } from "@/app/element/ui/DeclIconButton";
import type { JSX } from "solid-js";
import { createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";

/** A window that can take a docked pane: its host label and its backend window id. */
export interface RedockTarget {
    label: string;
    windowId: string;
}

/** A floater's label, from its URL (`?windowLabel=floating-…`), or null in any other window. */
export function floatingWindowLabel(): string | null {
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "";
    return label.startsWith("floating-") ? label : null;
}

/**
 * The window a floater redocks into: the main window, else another full window
 * that has registered its backend id (a window that hasn't has `windowId: null`).
 * Never a floater or a browser-pane window. Null when there is none, which hides
 * the button: there is nowhere to dock to.
 */
export function pickRedockTarget(
    instances: ReadonlyArray<{ label: string; windowId: string | null }>
): RedockTarget | null {
    const usable = instances.filter(
        (i): i is { label: string; windowId: string } =>
            !!i.windowId && !i.label.startsWith("floating-") && !i.label.startsWith("browser-pane-")
    );
    return usable.find((i) => i.label === "main") ?? usable[0] ?? null;
}

export async function findRedockTarget(): Promise<RedockTarget | null> {
    try {
        return pickRedockTarget(await getApi().listWindowInstances());
    } catch (e) {
        console.error("[floating-redock] could not list windows", e);
        return null;
    }
}

/** Where a redock put the pane: the workspace of the window that shows it, and how it was placed
 *  (`fallback`: no origin was found, so it went into the target's active tab). */
export interface RedockResult {
    workspaceId: string;
    placed: "stack" | "neighbour" | "fallback";
}

/**
 * Redock this floater's pane(s) into `target`'s active tab: the same saga and
 * the same on-demand lookup the drag redock uses (floating-pane-workspace.tsx).
 * `reloadMuxObject`, not `loadAndPinMuxObject`: an async click handler has no
 * matching unpin. The saga empties the floater's tab and the empty-tab watcher
 * closes the floater. Returns whether it redocked.
 */
export async function redockFloaterInto(target: RedockTarget): Promise<boolean> {
    return (await redockFloater(target, false)) != null;
}

/**
 * `redockFloaterInto`, and with `toOrigin` the saga puts each pane back where it was torn from when that
 * place can still be found, with `target`'s active tab only the fallback. Resolves to where the first pane
 * went, or null when nothing was redocked.
 */
async function redockFloater(target: RedockTarget, toOrigin: boolean): Promise<RedockResult | null> {
    const sourceWs = atoms.workspace();
    const sourceTabId = atoms.activeTabId();
    if (!sourceWs || !sourceTabId) return null;
    let targetWs: Workspace;
    try {
        const targetWindow = await MOS.reloadMuxObject<MuxWindow>(MOS.makeORef("window", target.windowId));
        targetWs = await MOS.reloadMuxObject<Workspace>(MOS.makeORef("workspace", targetWindow.workspaceid));
    } catch (e) {
        console.error("[floating-redock] failed to resolve the target window's workspace", e);
        return null;
    }
    if (!targetWs.activetabid || !targetWs.oid) {
        console.warn("[floating-redock] the target window has no active tab");
        return null;
    }
    // The floater's panes (a pane torn off with several tabs goes back whole).
    const panes = floaterPanes(sourceTabId);
    if (panes.length === 0) return null;
    let result: RedockResult = { workspaceId: targetWs.oid, placed: "fallback" };
    try {
        for (const [i, pane] of panes.entries()) {
            const answer = await redockFloaterPane(
                pane,
                { tabId: sourceTabId, workspaceId: sourceWs.oid },
                { tabId: targetWs.activetabid, workspaceId: targetWs.oid },
                undefined,
                toOrigin
            );
            if (i === 0 && answer?.placed && answer.target_workspace_id) {
                result = { workspaceId: answer.target_workspace_id, placed: answer.placed };
            }
        }
    } catch (e) {
        console.error("[floating-redock] RedockFloatingPane failed", e);
        return null;
    }
    return result;
}

/** The label of the window that shows `workspaceId`, else `fallback` (the window the redock was aimed at). */
async function windowLabelShowing(workspaceId: string, fallback: string): Promise<string> {
    try {
        for (const inst of await getApi().listWindowInstances()) {
            if (!inst.windowId || inst.label.startsWith("floating-") || inst.label.startsWith("browser-pane-")) continue;
            const win = await MOS.reloadMuxObject<MuxWindow>(MOS.makeORef("window", inst.windowId));
            if (win?.workspaceid === workspaceId) return inst.label;
        }
    } catch (e) {
        console.error("[floating-redock] could not find the window that shows the pane", e);
    }
    return fallback;
}

/** Redock this floater: back where it was torn from when that place can still be found, else into the main
 *  window's active tab. The window that shows the pane comes forward so the user sees where it went (no focus
 *  is moved into the pane, and no tab is switched). The button and the `pane:redock` command both run this.
 *  SPEC_FLOATING_PANE_REDOCK_TO_ORIGIN_2026_11_03.md */
export async function redockThisFloater(): Promise<boolean> {
    if (!floatingWindowLabel()) return false;
    const target = await findRedockTarget();
    if (!target) return false;
    // Hold the guard across the saga AND the focus call: the floater's empty-tab watcher
    // closes the window as soon as the tab empties, which is before the saga call returns
    // (the backend broadcasts the tab update first), and a closed window runs no more code.
    // The guard releases at the end and the floater then closes.
    return holdRedock(async () => {
        const result = await redockFloater(target, true);
        if (!result) return false;
        const label = result.placed === "fallback" ? target.label : await windowLabelShowing(result.workspaceId, target.label);
        await getApi().focusWindow(label).catch(console.error);
        return true;
    });
}

/** Header Redock button for a floating pane. Hidden when no window can take the pane; the check
 *  runs on mount and when the floater gets focus again (a window may have opened or closed). */
export function FloatingRedockButton(): JSX.Element {
    const [target, setTarget] = createSignal<RedockTarget | null>(null);
    const refresh = () => void findRedockTarget().then(setTarget);
    onMount(() => {
        refresh();
        window.addEventListener("focus", refresh);
        onCleanup(() => window.removeEventListener("focus", refresh));
    });
    const decl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: <PaneRestoreGlyph />,
        title: "Redock",
        click: () => {
            void redockThisFloater().then((ok) => {
                if (!ok) refresh(); // the target may have gone: hide the button if so
            });
        },
    };
    const shown = createMemo(() => target() != null);
    return (
        <Show when={shown()}>
            <DeclIconButton decl={decl} className="block-frame-redock" />
        </Show>
    );
}
