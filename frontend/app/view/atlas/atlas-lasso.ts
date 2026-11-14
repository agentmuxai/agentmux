// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The box a drag draws over the Atlas map to select agents. A drag on the background pans the map; with Shift
 * held, or with Select on, it draws this box instead (and Shift then pans). The nodes it touches are shown as it
 * moves, and are handed over when it is let go. Escape, or a cancelled pointer, drops it and selects nothing.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import type { Placed } from "./atlas-layout";
import { isDrag, nodesInRect, rectBetween, toMapRect, type Rect } from "./atlas-select";
import type { Viewport } from "./atlas-viewport";

export interface AtlasLasso {
    /** The box being drawn, in the pane's pixels; null when none is. */
    rect: Accessor<Rect | null>;
    /** The keys of the nodes the box touches now. */
    touched: Accessor<ReadonlySet<string>>;
    /** Whether a press on the background at `e` draws a box rather than pans. */
    wants: (e: PointerEvent) => boolean;
    start: (e: PointerEvent, viewport: HTMLElement) => void;
    onPointerMove: (e: PointerEvent) => void;
    onPointerUp: (e: PointerEvent) => void;
    cancel: () => void;
}

const NONE: ReadonlySet<string> = new Set();

export function useAtlasLasso(opts: {
    /** Select is on: a plain drag draws the box. */
    selecting: Accessor<boolean>;
    view: Accessor<Viewport>;
    nodes: Accessor<ReadonlyMap<string, Placed>>;
    /** The nodes the box touched when it was let go. */
    onPick: (keys: string[]) => void;
}): AtlasLasso {
    const [rect, setRect] = createSignal<Rect | null>(null);
    let drag: { id: number; sx: number; sy: number; left: number; top: number } | null = null;

    const touchedKeys = (r: Rect | null) => (r && isDrag(r) ? nodesInRect(opts.nodes(), toMapRect(r, opts.view())) : []);
    const touched = createMemo<ReadonlySet<string>>(() => {
        const keys = touchedKeys(rect());
        return keys.length > 0 ? new Set(keys) : NONE;
    });
    const cancel = () => {
        drag = null;
        setRect(null);
    };

    return {
        rect,
        touched,
        wants: (e) => e.button === 0 && opts.selecting() !== e.shiftKey,
        start: (e, viewport) => {
            const box = viewport.getBoundingClientRect();
            drag = { id: e.pointerId, sx: e.clientX - box.left, sy: e.clientY - box.top, left: box.left, top: box.top };
            viewport.setPointerCapture?.(e.pointerId);
            setRect(rectBetween(drag.sx, drag.sy, drag.sx, drag.sy));
        },
        onPointerMove: (e) => {
            if (!drag || e.pointerId !== drag.id) return;
            setRect(rectBetween(drag.sx, drag.sy, e.clientX - drag.left, e.clientY - drag.top));
        },
        onPointerUp: (e) => {
            if (!drag || e.pointerId !== drag.id) return;
            const keys = e.type === "pointercancel" ? [] : touchedKeys(rect());
            cancel();
            if (keys.length > 0) opts.onPick(keys);
        },
        cancel,
    };
}
