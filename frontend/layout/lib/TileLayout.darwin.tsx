// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS-specific TileLayout: the WKWebView hooks for TileLayout.core.tsx.
//
// WKWebView doesn't support pragmatic-dnd's dragHandle option (sets
// draggable="true/false" on child/parent which breaks drag). The core
// registers draggable() directly on the header element instead
// (querySelector '[data-role="block-header"]'), which avoids that.

import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import clsx from "clsx";
import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { debounce, throttle } from "throttle-debounce";
import { createTileLayout, type ResizeHandleComponentProps, type TileLayoutPlatform } from "./TileLayout.core";

export { tileItemType } from "./TileLayout.core";
export type { TileLayoutProps } from "./TileLayout.core";

const ResizeHandle = (props: ResizeHandleComponentProps) => {
    let resizeHandleRef: HTMLDivElement | undefined;
    const [trackingPointer, setTrackingPointer] = createSignal<number | undefined>(undefined);

    // WKWebView does not apply CSS cursor on transformed absolute elements.
    // Work around by setting document.body.style.cursor directly.
    const cursorStyle = () => props.resizeHandleProps.flexDirection === "row" ? "ew-resize" : "ns-resize";

    const handlePointerMove = throttle(10, (event: PointerEvent) => {
        if (trackingPointer() === event.pointerId) {
            const { clientX, clientY } = event;
            // Flipped defaults (SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26 §2):
            // plain drag = group resize, Shift+drag = direct 2-node transfer.
            props.layoutModel.onResizeMove(props.resizeHandleProps, clientX, clientY, !event.shiftKey);
        }
    });

    function onPointerDown(event: PointerEvent) {
        resizeHandleRef?.setPointerCapture(event.pointerId);
    }

    function onPointerCapture(event: PointerEvent) {
        setTrackingPointer(event.pointerId);
        document.body.style.cursor = cursorStyle();
    }

    const onPointerRelease = debounce(30, (event: PointerEvent) => {
        setTrackingPointer(undefined);
        document.body.style.cursor = "";
        props.layoutModel.onResizeEnd();
    });

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            onGotPointerCapture={onPointerCapture}
            onLostPointerCapture={onPointerRelease}
            onMouseEnter={() => { document.body.style.cursor = cursorStyle(); }}
            onMouseLeave={() => { document.body.style.cursor = ""; }}
            style={{
                ...props.resizeHandleProps.transform as JSX.CSSProperties,
                cursor: cursorStyle(),
                "pointer-events": "auto",
                "z-index": "var(--zindex-layout-resize-handle)",
            }}
            onPointerMove={handlePointerMove}
        >
            <div class="line" />
        </div>
    );
};

const darwin: TileLayoutPlatform = {
    animateDelayMs: 50,
    boundsCheckRequiresTileDrag: false,
    clearDraggingWhenInactive: false,

    beforeDragStart: () => {
        // Suppress WebKit's "drop rejected" snapback: a pane
        // tear-off releases the drag OUTSIDE any pragmatic-dnd
        // drop target (the floater is created on `dragend`), so
        // the browser would otherwise animate the drag preview
        // back into the source window ("it doesn't want to go").
        // preventUnhandled makes every element a drop target in
        // the browser's eyes, so the drop is "handled" and there's
        // no snapback — the preview just vanishes on release.
        // In-window rearrange still works via pragmatic-dnd's own
        // drop targets. Stopped in onDrop.
        preventUnhandled.start();
    },
    beforeDrop: () => {
        preventUnhandled.stop();
    },

    ResizeHandle,
};

export const TileLayout = createTileLayout(darwin);
