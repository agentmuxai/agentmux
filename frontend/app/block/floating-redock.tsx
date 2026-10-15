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
import { WorkspaceService } from "@/app/store/services";
import { atoms, getApi } from "@/store/global";
import { IconButton } from "@/element/iconbutton";
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

/**
 * Redock this floater's pane(s) into `target`'s active tab: the same saga and
 * the same on-demand lookup the drag redock uses (floating-pane-workspace.tsx).
 * `reloadMuxObject`, not `loadAndPinMuxObject`: an async click handler has no
 * matching unpin. The saga empties the floater's tab and the empty-tab watcher
 * closes the floater. Returns whether it redocked.
 */
export async function redockFloaterInto(target: RedockTarget): Promise<boolean> {
    const sourceWs = atoms.workspace();
    const sourceTabId = atoms.activeTabId();
    if (!sourceWs || !sourceTabId) return false;
    let targetWs: Workspace;
    try {
        const targetWindow = await MOS.reloadMuxObject<MuxWindow>(MOS.makeORef("window", target.windowId));
        targetWs = await MOS.reloadMuxObject<Workspace>(MOS.makeORef("workspace", targetWindow.workspaceid));
    } catch (e) {
        console.error("[floating-redock] failed to resolve the target window's workspace", e);
        return false;
    }
    if (!targetWs.activetabid || !targetWs.oid) {
        console.warn("[floating-redock] the target window has no active tab");
        return false;
    }
    // Non-reactive read (no owner in a click handler): the floater's tab, and every block in it.
    const sourceTab = MOS.getObjectValue<Tab>(MOS.makeORef("tab", sourceTabId));
    const blockIds = sourceTab?.blockids ?? [];
    if (blockIds.length === 0) return false;
    try {
        for (const blockId of blockIds) {
            await WorkspaceService.RedockFloatingPane(
                blockId,
                sourceTabId,
                sourceWs.oid,
                targetWs.activetabid,
                targetWs.oid
            );
        }
    } catch (e) {
        console.error("[floating-redock] RedockFloatingPane failed", e);
        return false;
    }
    return true;
}

/** Redock this floater into the main window, and bring that window forward so the user sees where
 *  the pane went (no focus is moved into the pane). The button and the `pane:redock` command both run this. */
export async function redockThisFloater(): Promise<boolean> {
    if (!floatingWindowLabel()) return false;
    const target = await findRedockTarget();
    if (!target) return false;
    if (!(await redockFloaterInto(target))) return false;
    await getApi().focusWindow(target.label).catch(console.error);
    return true;
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
            <IconButton decl={decl} className="block-frame-redock" />
        </Show>
    );
}
