// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific TileLayout: the WebView2 hooks for TileLayout.core.tsx.
//
// dragHandle: undefined — whole-tile drag (pragmatic-dnd dragHandle breaks
// WebView2: it sets draggable="true" on the handle AND draggable="false" on
// the tile, and WebView2 does not fire dragstart from a draggable="true"
// child inside a draggable="false" parent — breaking pane DnD entirely on
// Windows). The core registers draggable() directly on the header instead.

import { notifyPaneReflow } from "@/app/platform/pane-anim";
import clsx from "clsx";
import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { debounce, throttle } from "throttle-debounce";
import { isInResizeHandleZone } from "./layoutGeometry";
import { createTileLayout, type ResizeHandleComponentProps, type TileLayoutPlatform } from "./TileLayout.core";

export type { TileLayoutProps } from "./TileLayout.core";

const ResizeHandle = (props: ResizeHandleComponentProps) => {
    let resizeHandleRef: HTMLDivElement | undefined;
    const [trackingPointer, setTrackingPointer] = createSignal<number | undefined>(undefined);

    const handlePointerMove = throttle(10, (event: PointerEvent) => {
        if (trackingPointer() === event.pointerId) {
            const { clientX, clientY } = event;
            // Flipped defaults (SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26 §2):
            // plain drag = group resize, Shift+drag = direct 2-node transfer.
            props.layoutModel.onResizeMove(props.resizeHandleProps, clientX, clientY, !event.shiftKey);
        }
    });

    function onPointerDown(event: PointerEvent) {
        // Prevent mousedown (and thus dragstart) from reaching elements below
        // the resize handle — stops a border press from triggering a tear-off.
        event.preventDefault();
        resizeHandleRef?.setPointerCapture(event.pointerId);
    }

    function onPointerCapture(event: PointerEvent) {
        setTrackingPointer(event.pointerId);
    }

    const onPointerRelease = debounce(30, (event: PointerEvent) => {
        setTrackingPointer(undefined);
        props.layoutModel.onResizeEnd();
    });

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            onGotPointerCapture={onPointerCapture}
            onLostPointerCapture={onPointerRelease}
            style={props.resizeHandleProps.transform as JSX.CSSProperties}
            onPointerMove={handlePointerMove}
        >
            <div class="line" />
        </div>
    );
};

const win32: TileLayoutPlatform = {
    // Issue #774 / SPEC_TAB_CONTENT_REVEAL_GATE: the prior 50 ms was
    // too short. Block measurements (block.tsx `getBoundingClientRect`,
    // virtual-list `measureElement`) haven't completed at the 50 ms
    // mark, so transitions enable mid-settle and panes snap-then-
    // animate. Bumped to 150 ms — gives the post-paint measurement
    // wave time to complete before transitions kick in. Combined with
    // the reveal gate at the workspace level, the user sees neither
    // the snap nor the partial paint.
    animateDelayMs: 150,

    boundsCheckRequiresTileDrag: false,

    // Native browser panes read the settle signal to re-sample + SetWindowPos
    // their HWND onto the new rect. See SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.
    onLeafGeometryChange: notifyPaneReflow,

    // Handles a drag the safety net ends (drag-session.ts), where onDrop never fires and
    // setIsDragging(false) is never called directly.
    clearDraggingWhenInactive: true,

    rejectDragAt: (layoutModel, input) => {
        // Reject drags that originate inside a resize-handle zone.
        // The resize handle overlaps slightly with the adjacent
        // header; this guard ensures a near-border mousedown never
        // turns into a tear-off even if the pointer just missed the
        // handle element.
        const containerEl = layoutModel.displayContainerRef?.current;
        if (!containerEl) return false;
        const containerRect = containerEl.getBoundingClientRect();
        const localX = input.clientX - containerRect.left;
        const localY = input.clientY - containerRect.top;
        return isInResizeHandleZone(layoutModel.resizeHandles(), layoutModel.resizeHandleSizePx() / 2, localX, localY);
    },

    ResizeHandle,
};

export const TileLayout = createTileLayout(win32);
