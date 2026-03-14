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

import { getApi } from "@/store/global";
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
            Logger.info("dnd:overlay", "cross-drag ended", { dragId: event.payload.dragId, result: event.payload.result });
            setIsTarget(false);
            setDragType(null);
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
