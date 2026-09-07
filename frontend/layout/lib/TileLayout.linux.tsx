// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Linux-specific TileLayout: the WebKitGTK / CEF-on-Wayland hooks for
// TileLayout.core.tsx.
//
// WebKitGTK does not support HTML5 DnD from draggable="true" child inside an
// explicit draggable="false" parent — pragmatic-dnd's dragHandle option sets
// exactly that, so we cannot use it. The fix (same as win32) is to register
// draggable() directly on the header element, which the core does. Only the
// header gets draggable="true"; the tile root receives no draggable attribute
// (implicitly non-draggable). No explicit draggable="false" on any parent →
// no WebKitGTK breakage, and drag is correctly restricted to the header.

import { getApi } from "@/app/store/global";
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import clsx from "clsx";
import { onCleanup } from "solid-js";
import type { JSX } from "solid-js";
import { throttle } from "throttle-debounce";
import { createTileLayout, type ResizeHandleComponentProps, type TileLayoutPlatform } from "./TileLayout.core";

export { tileItemType } from "./TileLayout.core";
export type { TileLayoutProps } from "./TileLayout.core";

const ResizeHandle = (props: ResizeHandleComponentProps) => {
    let resizeHandleRef: HTMLDivElement | undefined;

    // CEF/Chromium on Linux (Wayland) does NOT keep a stable PointerEvent.pointerId
    // across a press → drag: the pointerdown arrives as one id (e.g. 1) but the
    // subsequent pointermove events arrive as a *different* id (e.g. 2), and
    // setPointerCapture() does not reliably route those moves once the cursor
    // leaves the thin handle. The pointerId-match + onGotPointerCapture approach
    // used on win32/darwin therefore never fires onResizeMove here — verified via
    // logging: every move logged `tracking=1 id=2 match=false`, so the guard was
    // always false and the divider never moved.
    //
    // Fix: don't depend on pointerId or pointer-capture routing. Arm on pointerdown
    // and listen for moves on `window`, which receives every pointermove regardless
    // of pointerId or whether the cursor is still over the handle. End on the
    // primary-button release (pointerup/pointercancel, or buttons going to 0).
    let teardown: (() => void) | null = null;

    function onPointerDown(event: PointerEvent) {
        if (event.button !== 0 || teardown) return; // primary button, not already resizing
        // Keep the press from also starting a tile tear-off or native window drag.
        event.preventDefault();
        // Best-effort capture: helps keep moves flowing over native panes on
        // platforms where capture works; on Linux/CEF the unstable id makes it
        // unreliable, which is why the window listeners below drive the resize.
        try {
            resizeHandleRef?.setPointerCapture(event.pointerId);
        } catch {
            /* ignore — capture is best-effort */
        }

        const onMove = throttle(10, (e: PointerEvent) => {
            if (!teardown) return; // trailing throttle call after release
            if ((e.buttons & 1) === 0) {
                teardown(); // primary button released without a pointerup reaching us
                return;
            }
            // Flipped defaults (SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26 §2):
            // plain drag = group resize, Shift+drag = direct 2-node transfer.
            props.layoutModel.onResizeMove(props.resizeHandleProps, e.clientX, e.clientY, !e.shiftKey);
        });
        const onUp = () => teardown?.();

        teardown = () => {
            window.removeEventListener("pointermove", onMove);
            window.removeEventListener("pointerup", onUp);
            window.removeEventListener("pointercancel", onUp);
            onMove.cancel();
            teardown = null;
            props.layoutModel.onResizeEnd();
        };

        window.addEventListener("pointermove", onMove);
        window.addEventListener("pointerup", onUp);
        window.addEventListener("pointercancel", onUp);
    }

    onCleanup(() => teardown?.());

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            style={props.resizeHandleProps.transform as JSX.CSSProperties}
        >
            <div class="line" />
        </div>
    );
};

const linux: TileLayoutPlatform = {
    animateDelayMs: 50,

    // Guard: only fire during an active tile drag (dragState.nodeId set). Tab DnD must not
    // trigger persistToBackend via treeReducer, which caused a crash on Linux (see drag.rs notes).
    boundsCheckRequiresTileDrag: true,

    clearDraggingWhenInactive: false,

    beforeDragStart: () => {
        // Suppress WebKitGTK's "drop rejected" snapback — a pane
        // tear-off releases outside any pragmatic-dnd drop target
        // (floater created on dragend), so the browser would
        // otherwise animate the drag preview back into the source
        // window. preventUnhandled makes the drop "handled" so the
        // preview just vanishes on release. In-window rearrange
        // still works via pragmatic-dnd's own drop targets.
        preventUnhandled.start();
    },
    afterDragStart: () => {
        getApi().setJsDragActive(true).catch(() => {});
    },
    beforeDrop: () => {
        preventUnhandled.stop();
    },
    afterDrop: () => {
        getApi().setJsDragActive(false).catch(() => {});
    },

    ResizeHandle,
};

export const TileLayout = createTileLayout(linux);
