// Copyright 2026, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { isLinux } from "@/util/platformutil";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback } from "react";

// Excluded from drag: clicks on interactive elements or elements explicitly
// marked with data-tauri-drag-region="false" (used by action-widgets, window
// buttons, tab bar, etc.).
const DRAG_EXCLUDE_SELECTOR = 'button, input, a, [data-tauri-drag-region="false"]';

/**
 * Centralized window drag hook. Returns props to spread on a draggable element.
 *
 * On Linux, drag is handled natively by drag.rs (GTK motion detection) —
 * JS-level startDragging() and data-tauri-drag-region both trigger an
 * immediate Wayland compositor pointer grab that swallows button clicks.
 * So on Linux the mousedown handler is a no-op and no drag attributes are set.
 *
 * On macOS/Windows, returns startDragging() mousedown handler +
 * data-tauri-drag-region attribute.
 */
export function useWindowDrag(): {
    dragProps: Record<string, unknown>;
    onMouseDown: (e: React.MouseEvent) => void;
} {
    const onMouseDown = useCallback((e: React.MouseEvent) => {
        if (isLinux()) return;
        if (e.button !== 0) return;
        const target = e.target as HTMLElement;
        if (target.closest(DRAG_EXCLUDE_SELECTOR)) return;
        e.preventDefault();
        getCurrentWindow().startDragging().catch(() => {});
    }, []);

    const dragProps = isLinux() ? {} : { "data-tauri-drag-region": true };

    return { dragProps, onMouseDown };
}
