// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Reordering the tray's tiles with the pointer: drag a tile onto another and it lands before or after it,
 * by which half the pointer is over. The keyboard way is Alt+Left and Alt+Right on a focused tile
 * (attachment-tray-state.ts). The numbers on the tiles and the numbered list the agent receives follow the
 * order, because both are read from the draft's items.
 *
 * The drag carries its own type, never `Files`, so the window's file-drop controller (app/drag/file-drop.ts)
 * does not take it for a drop from the desktop and draws no overlay over the composer.
 */

import { createSignal } from "solid-js";
import type { AttachmentDraft } from "./attachment-draft";

export const REORDER_MIME = "application/x-agentmux-attachment";

/** What a tile needs to take part in a reorder drag. */
export interface TileReorder {
    dragging: boolean;
    /** Which edge of this tile the held one would land on. */
    drop?: "before" | "after";
    onDragStart: (e: DragEvent) => void;
    onDragOver: (e: DragEvent) => void;
    onDrop: (e: DragEvent) => void;
    onDragEnd: () => void;
}

/** "Moved name to position 2 of 5.", read out after a drag or an Alt+arrow. */
export function movedAnnouncement(d: AttachmentDraft, key: string): string | null {
    const items = d.items();
    const at = items.findIndex((i) => i.key === key);
    return at < 0 ? null : `Moved ${items[at].name} to position ${at + 1} of ${items.length}.`;
}

export function createTileReorder(d: AttachmentDraft, announce: (text: string) => void) {
    const [held, setHeld] = createSignal<string | null>(null);
    const [over, setOver] = createSignal<{ key: string; after: boolean } | null>(null);
    const clear = () => {
        setHeld(null);
        setOver(null);
    };

    return (key: string): TileReorder => ({
        get dragging() {
            return held() === key;
        },
        get drop() {
            const o = over();
            return o && o.key === key && held() !== key ? (o.after ? "after" : "before") : undefined;
        },
        onDragStart(e) {
            e.dataTransfer?.setData(REORDER_MIME, key);
            if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
            setHeld(key);
        },
        onDragOver(e) {
            if (held() === null) return;
            e.preventDefault();
            if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            const after = e.clientX > r.left + r.width / 2;
            const o = over();
            if (!o || o.key !== key || o.after !== after) setOver({ key, after });
        },
        onDrop(e) {
            const from = held();
            const o = over();
            if (from === null) return;
            e.preventDefault();
            e.stopPropagation();
            clear();
            if (from === key) return;
            d.moveNear(from, key, o?.key === key ? o.after : false);
            const said = movedAnnouncement(d, from);
            if (said) announce(said);
        },
        onDragEnd: clear,
    });
}
