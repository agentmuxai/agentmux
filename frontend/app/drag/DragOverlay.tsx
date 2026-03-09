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
import { memo, useEffect, useState } from "react";
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

const DragOverlay = memo(() => {
    const [isTarget, setIsTarget] = useState(false);
    const [dragType, setDragType] = useState<"pane" | "tab" | null>(null);
    const [windowLabel, setWindowLabel] = useState<string | null>(null);

    useEffect(() => {
        getApi()
            .getWindowLabel()
            .then((label) => setWindowLabel(label));
    }, []);

    useEffect(() => {
        if (!windowLabel) return;

        let unlistenUpdate: (() => void) | null = null;
        let unlistenEnd: (() => void) | null = null;

        const api = getApi();

        Logger.debug("dnd:overlay", "DragOverlay listening for events", { windowLabel });

        api.listen("cross-drag-update", (event: { payload: CrossDragUpdateEvent }) => {
            const data = event.payload;
            if (data.targetWindow === windowLabel && data.sourceWindow !== windowLabel) {
                Logger.info("dnd:overlay", "this window is drop target", {
                    windowLabel,
                    dragId: data.dragId,
                    dragType: data.dragType,
                    sourceWindow: data.sourceWindow,
                    screenX: data.screenX,
                    screenY: data.screenY,
                });
                setIsTarget(true);
                setDragType(data.dragType);
            } else {
                if (isTarget) {
                    Logger.debug("dnd:overlay", "no longer drop target", { windowLabel, targetWindow: data.targetWindow });
                }
                setIsTarget(false);
                setDragType(null);
            }
        }).then((fn) => {
            unlistenUpdate = fn;
        });

        api.listen("cross-drag-end", (event: { payload: CrossDragEndEvent }) => {
            Logger.info("dnd:overlay", "cross-drag ended", { dragId: event.payload.dragId, result: event.payload.result });
            setIsTarget(false);
            setDragType(null);
        }).then((fn) => {
            unlistenEnd = fn;
        });

        // Also listen for cross-drag-start to clear stale state
        let unlistenStart: (() => void) | null = null;
        api.listen("cross-drag-start", () => {
            Logger.debug("dnd:overlay", "cross-drag started — resetting overlay state");
            // New drag started — reset state
            setIsTarget(false);
            setDragType(null);
        }).then((fn) => {
            unlistenStart = fn;
        });

        return () => {
            unlistenUpdate?.();
            unlistenEnd?.();
            unlistenStart?.();
        };
    }, [windowLabel]);

    if (!isTarget) return null;

    return (
        <div className="cross-drag-overlay">
            <div className="cross-drag-overlay-content">
                <i className={dragType === "tab" ? "fa fa-window-maximize" : "fa fa-th-large"} />
                <span>Drop {dragType === "tab" ? "tab" : "pane"} here</span>
            </div>
        </div>
    );
});

DragOverlay.displayName = "DragOverlay";

export { DragOverlay };
