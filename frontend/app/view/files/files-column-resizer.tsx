// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The divider on a list column header's right edge: drag it to resize the column, double-click to give the
 * column its own width back (files-columns.ts). It sits inside the header button and keeps its presses from
 * reaching it, so a drag or a click on the divider never sorts.
 */

import { createSignal, type JSX } from "solid-js";
import type { createColumnWidths } from "./files-columns";
import type { SortKey } from "./files-sort";

export function ColumnResizer(props: {
    colKey: SortKey;
    label: string;
    columns: ReturnType<typeof createColumnWidths>;
    /** The pane's zoom: the pointer moves in screen pixels, a width is in the pane's own. */
    zoom: () => number;
}): JSX.Element {
    const [dragging, setDragging] = createSignal(false);
    const onPointerDown = (e: PointerEvent): void => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        const handle = e.currentTarget as HTMLElement;
        const cell = handle.parentElement;
        if (!cell) return;
        const zoom = props.zoom() || 1;
        const startX = e.clientX;
        const startWidth = cell.getBoundingClientRect().width / zoom;
        handle.setPointerCapture?.(e.pointerId);
        setDragging(true);
        const move = (m: PointerEvent): void => props.columns.preview(props.colKey, startWidth + (m.clientX - startX) / zoom);
        const end = (): void => {
            handle.removeEventListener("pointermove", move);
            handle.removeEventListener("pointerup", end);
            handle.removeEventListener("pointercancel", end);
            setDragging(false);
            props.columns.commit(props.colKey);
        };
        handle.addEventListener("pointermove", move);
        handle.addEventListener("pointerup", end);
        handle.addEventListener("pointercancel", end);
    };
    return (
        <span
            class="files-col-resizer"
            classList={{ "files-col-resizer--active": dragging() }}
            role="separator"
            aria-orientation="vertical"
            aria-label={`Resize the ${props.label} column`}
            onPointerDown={onPointerDown}
            onClick={(e) => e.stopPropagation()}
            onDblClick={(e) => {
                e.stopPropagation();
                props.columns.reset(props.colKey);
            }}
        />
    );
}
