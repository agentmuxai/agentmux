// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DragOverlay
 *
 * Renders a full-window overlay when a cross-window drag is hovering
 * over this window. Shows a visual indicator that a drop will be accepted.
 *
 * Listens for "cross-drag-update" and "cross-drag-end" Tauri events.
 * Only visible when this window is the target of an active cross-drag.
 */

import { atoms, getApi } from "@/store/global";
import { WorkspaceService } from "@/app/store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { Logger } from "@/util/logger";
import { createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import "./drag-overlay.scss";

interface CrossDragUpdateEvent {
    dragId: string;
    dragType: "pane" | "tab";
    payload: { blockId?: string; tabId?: string };
    targetWindow: string | null;
    sourceWindow: string;
    screenX: number;
    screenY: number;
}

interface CrossDragEndEvent {
    dragId: string;
    result: "drop" | "tearoff" | "cancel";
    targetWindow: string | null;
    sourceWindow: string;
    dragType: "pane" | "tab";
    payload: { blockId?: string; tabId?: string };
    sourceWorkspaceId: string;
    sourceTabId: string;
}

function DragOverlay(): JSX.Element {
    const [isTarget, setIsTarget] = createSignal(false);
    const [dragType, setDragType] = createSignal<"pane" | "tab" | null>(null);
    const [windowLabel, setWindowLabel] = createSignal<string | null>(null);

    onMount(async () => {
        const label = await getApi().getWindowLabel();
        setWindowLabel(label);
    });

    // Set up event listeners once we have the window label
    const setupListeners = () => {
        const wl = windowLabel();
        if (!wl) return;

        const api = getApi();
        Logger.debug("dnd:overlay", "DragOverlay listening for events", { windowLabel: wl });

        let unlistenUpdate: (() => void) | null = null;
        let unlistenEnd: (() => void) | null = null;
        let unlistenStart: (() => void) | null = null;

        api.listen("cross-drag-update", (event: { payload: CrossDragUpdateEvent }) => {
            const data = event.payload;
            if (data.targetWindow === wl && data.sourceWindow !== wl) {
                setIsTarget(true);
                setDragType(data.dragType);
            } else {
                setIsTarget(false);
                setDragType(null);
            }
        }).then((fn) => { unlistenUpdate = fn; });

        api.listen("cross-drag-end", (event: { payload: CrossDragEndEvent }) => {
            const data = event.payload;
            Logger.info("dnd:overlay", "cross-drag ended", { dragId: data.dragId, result: data.result, targetWindow: data.targetWindow });
            setIsTarget(false);
            setDragType(null);

            // Handle drop onto this window: move the item into our active workspace/tab
            if (data.result === "drop" && data.targetWindow === wl) {
                const myWsId = atoms.workspace()?.oid;
                const myActiveTabId = atoms.activeTabId();
                if (!myWsId || !myActiveTabId) {
                    Logger.warn("dnd:overlay", "cross-window drop: no active workspace/tab", { myWsId, myActiveTabId });
                    return;
                }

                if (data.dragType === "pane" && data.payload.blockId) {
                    Logger.info("dnd:overlay", "cross-window drop: moving pane into this window", {
                        blockId: data.payload.blockId,
                        sourceTabId: data.sourceTabId,
                        destTabId: myActiveTabId,
                    });
                    WorkspaceService.MoveBlockToTab(myWsId, data.payload.blockId, data.sourceTabId, myActiveTabId, true).catch((e) => {
                        Logger.error("dnd:overlay", "MoveBlockToTab failed", { error: String(e) });
                    });
                } else if (data.dragType === "tab" && data.payload.tabId) {
                    Logger.info("dnd:overlay", "cross-window drop: moving tab into this window", {
                        tabId: data.payload.tabId,
                        sourceWsId: data.sourceWorkspaceId,
                        destWsId: myWsId,
                    });
                    WorkspaceService.MoveTabToWorkspace(data.payload.tabId, data.sourceWorkspaceId, myWsId)
                        .then(() => {
                            deleteLayoutModelForTab(data.payload.tabId);
                        })
                        .catch((e) => {
                            Logger.error("dnd:overlay", "MoveTabToWorkspace failed", { error: String(e) });
                        });
                }
            }
        }).then((fn) => { unlistenEnd = fn; });

        api.listen("cross-drag-start", () => {
            setIsTarget(false);
            setDragType(null);
        }).then((fn) => { unlistenStart = fn; });

        onCleanup(() => {
            unlistenUpdate?.();
            unlistenEnd?.();
            unlistenStart?.();
        });
    };

    // Run setup once window label is available
    onMount(() => {
        // Poll until label is set (it's set async in onMount above)
        const interval = setInterval(() => {
            if (windowLabel()) {
                clearInterval(interval);
                setupListeners();
            }
        }, 10);
        onCleanup(() => clearInterval(interval));
    });

    return (
        <Show when={isTarget()}>
            <div class="cross-drag-overlay">
                <div class="cross-drag-overlay-content">
                    <i class={dragType() === "tab" ? "fa fa-window-maximize" : "fa fa-th-large"} />
                    <span>Drop {dragType() === "tab" ? "tab" : "pane"} here</span>
                </div>
            </div>
        </Show>
    );
}

export { DragOverlay };
