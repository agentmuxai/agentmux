// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether an in-app element drag is in flight: a whole pane (tile), a Pane
 * Tab pill, or a Window Tab. Any pragmatic-dnd `draggable()` counts.
 *
 * For surfaces that can't see the drag themselves. A browser page is a
 * native window drawn above the DOM, so while the pointer is over it the OS
 * hands the drag to that page, which refuses it (a circle-slash cursor), and
 * the pane's own drop target underneath never fires. The browser view reads
 * this to open a hole through its page for the length of the drag
 * (browser-view.tsx's drag catcher).
 */

import { monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createRoot, createSignal, type Accessor } from "solid-js";

let inFlight: Accessor<boolean> | undefined;

/** Reactive; true from drag start until that drag's drop or cancel. */
export function elementDragInFlight(): boolean {
    if (!inFlight) {
        // One monitor for the app's lifetime, registered on first read so
        // importing this module has no side effects.
        inFlight = createRoot(() => {
            const [get, set] = createSignal(false);
            monitorForElements({
                onDragStart: () => set(true),
                // Fires on cancel too (Escape, dropped outside every target).
                onDrop: () => set(false),
            });
            // Windows can swallow pragmatic's own drop when snap layouts or
            // Alt+Tab interrupts a drag (TileLayout.win32.tsx's same safety
            // net). A stuck `true` would leave every browser page hidden.
            window.addEventListener("dragend", () => set(false));
            return get;
        });
    }
    return inFlight();
}
