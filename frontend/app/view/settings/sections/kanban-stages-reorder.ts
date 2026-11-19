// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reordering the stage rows with the pointer: hold a row's grip and drop it on another row, where it lands before or
// after that row by which half the pointer is over. The up and down buttons stay for the keyboard. The drag carries its
// own type, never `Files`, so the window's file-drop controller takes it for nothing from the desktop.

import { createSignal } from "solid-js";

export const STAGE_MIME = "application/x-agentmux-kanban-stage";

/** What a row needs to take part in a reorder drag; the grip takes `onDragStart` and `onDragEnd`, the row the rest. */
export interface StageReorder {
    dragging: boolean;
    /** Which edge of this row the held one would land on. */
    drop?: "before" | "after";
    onDragStart: (e: DragEvent) => void;
    onDragOver: (e: DragEvent) => void;
    onDrop: (e: DragEvent) => void;
    onDragEnd: () => void;
}

export function createStageReorder(moveNear: (id: string, target: string, after: boolean) => void) {
    const [held, setHeld] = createSignal<string | null>(null);
    const [over, setOver] = createSignal<{ id: string; after: boolean } | null>(null);
    const clear = () => {
        setHeld(null);
        setOver(null);
    };

    return (id: string): StageReorder => ({
        get dragging() {
            return held() === id;
        },
        get drop() {
            const o = over();
            return o && o.id === id && held() !== id ? (o.after ? "after" : "before") : undefined;
        },
        onDragStart(e) {
            const row = (e.currentTarget as HTMLElement).closest("li");
            if (e.dataTransfer) {
                e.dataTransfer.setData(STAGE_MIME, id);
                e.dataTransfer.effectAllowed = "move";
                if (row) e.dataTransfer.setDragImage(row, 12, 12);
            }
            setHeld(id);
        },
        onDragOver(e) {
            if (held() === null) return;
            e.preventDefault();
            if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            const after = e.clientY > r.top + r.height / 2;
            const o = over();
            if (!o || o.id !== id || o.after !== after) setOver({ id, after });
        },
        onDrop(e) {
            const from = held();
            const o = over();
            if (from === null) return;
            e.preventDefault();
            e.stopPropagation();
            clear();
            if (from !== id) moveNear(from, id, o?.id === id ? o.after : false);
        },
        onDragEnd: clear,
    });
}
