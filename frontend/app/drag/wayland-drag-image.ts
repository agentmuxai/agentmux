// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * No drag picture for tear-off-capable drags on native Wayland.
 *
 * A tear-off ends with a release outside every window. The compositor (GNOME's
 * Mutter) takes that as a rejected drop and slides the drag's picture back to
 * where the drag started, fading it, while the new window opens: a ghost that
 * says "invalid" for something that worked. The page can't make that drop
 * accepted (`preventUnhandled` only covers drops inside our own window), and
 * the picture can't be changed once the drag has started. A transparent
 * picture leaves the compositor nothing to animate.
 *
 * The cost: no picture follows the cursor during these drags on native
 * Wayland. Inside the window the drop highlight still shows where it will
 * land. X11, Windows and macOS keep their picture.
 */

import { isNativeWayland } from "@/app/host/window-system";

let transparent: HTMLCanvasElement | null = null;

/**
 * On native Wayland, give the drag a transparent picture and return true.
 * Elsewhere do nothing and return false. Call it where the drag's picture is
 * chosen (pragmatic-dnd's `onGenerateDragPreview`).
 */
export function hideDragImageOnNativeWayland(
    setDragImage: ((image: Element, x: number, y: number) => void) | null | undefined
): boolean {
    if (!setDragImage || !isNativeWayland()) return false;
    if (!transparent) {
        transparent = document.createElement("canvas");
        transparent.width = 1;
        transparent.height = 1;
    }
    setDragImage(transparent, 0, 0);
    return true;
}
