// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useWidgetDragReorder — drag-to-reorder state machine for pinned widgets.
 * Persists the new order to `widget:pinned` on drop. Extracted from
 * action-widgets.tsx.
 *
 * State machine: idle → pending (left-button down) → dragging (threshold exceeded)
 * A single DragState signal avoids ref/signal duplication.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";
import { createEffect, createSignal, onCleanup } from "solid-js";

const DRAG_THRESHOLD = 5;

type DragState =
    | { phase: "idle" }
    | { phase: "pending"; key: string; startX: number; startY: number; pointerId: number }
    | { phase: "dragging"; key: string; dropIndex: number };

const DRAG_IDLE: DragState = { phase: "idle" };

export function useWidgetDragReorder(opts: {
    containerRef: () => HTMLDivElement | undefined;
    pinnedWidgets: () => { key: string; widget: WidgetConfigType }[];
}) {
    const { containerRef, pinnedWidgets } = opts;

    const [dragState, setDragState] = createSignal<DragState>(DRAG_IDLE);

    const draggingKey = () => { const s = dragState(); return s.phase === "dragging" ? s.key      : null; };
    const dropIndex   = () => { const s = dragState(); return s.phase === "dragging" ? s.dropIndex : null; };

    // Cancel drag if the OS interrupts the gesture or the window loses visibility
    // (e.g. Alt+Tab with a button held). pointerup during a normal drop is handled
    // by the element's handlePointerUp — no global pointerup listener needed because
    // setPointerCapture redirects it to the element once dragging begins.
    createEffect(() => {
        if (dragState().phase === "idle") return;
        const cancel = () => setDragState(DRAG_IDLE);
        window.addEventListener("pointercancel",    cancel, { capture: true });
        window.addEventListener("visibilitychange", cancel);
        onCleanup(() => {
            window.removeEventListener("pointercancel",    cancel, { capture: true });
            window.removeEventListener("visibilitychange", cancel);
        });
    });

    const handlePointerDown = (key: string, e: PointerEvent) => {
        if (e.button !== 0) return;
        setDragState({ phase: "pending", key, startX: e.clientX, startY: e.clientY, pointerId: e.pointerId });
    };

    const handlePointerMove = (e: PointerEvent) => {
        const state = dragState();
        if (state.phase === "idle") return;
        if (state.phase === "pending") {
            const dx = e.clientX - state.startX;
            const dy = e.clientY - state.startY;
            if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
            (e.currentTarget as HTMLElement).setPointerCapture(state.pointerId);
        }
        e.preventDefault();
        const container = containerRef();
        if (!container) return;
        const slots = Array.from(container.querySelectorAll<HTMLElement>("[data-widget-slot]"));
        let newIndex = slots.length;
        for (let i = 0; i < slots.length; i++) {
            const rect = slots[i].getBoundingClientRect();
            if (e.clientX <= rect.right) {
                newIndex = e.clientX <= rect.left + rect.width / 2 ? i : i + 1;
                break;
            }
        }
        if (state.phase === "pending") {
            setDragState({ phase: "dragging", key: state.key, dropIndex: newIndex });
        } else if (state.dropIndex !== newIndex) {
            setDragState({ ...state, dropIndex: newIndex });
        }
    };

    const handlePointerUp = (_e: PointerEvent) => {
        const state = dragState();
        setDragState(DRAG_IDLE);
        if (state.phase !== "dragging") return;
        const current = pinnedWidgets();
        const shortNames = current.map(({ key }) => key.replace("defwidget@", ""));
        const dragShort = state.key.replace("defwidget@", "");
        const fromIdx = shortNames.indexOf(dragShort);
        if (fromIdx === -1) return;
        const next = [...shortNames];
        next.splice(fromIdx, 1);
        const adjustedDrop = fromIdx < state.dropIndex ? state.dropIndex - 1 : state.dropIndex;
        next.splice(adjustedDrop, 0, dragShort);
        if (next.join(",") !== shortNames.join(",")) {
            fireAndForget(async () => {
                await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": next } as any);
            });
        }
    };

    const handlePointerCancel = () => setDragState(DRAG_IDLE);

    return {
        draggingKey,
        dropIndex,
        handlePointerDown,
        handlePointerMove,
        handlePointerUp,
        handlePointerCancel,
    };
}
