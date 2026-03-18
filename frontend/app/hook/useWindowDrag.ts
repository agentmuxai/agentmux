// Copyright 2026, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { isLinux } from "@/util/platformutil";

/**
 * Centralized window drag hook. Returns props to spread on a draggable element.
 *
 * On Linux, drag is handled natively by drag.rs (GTK motion detection) —
 * data-tauri-drag-region triggers an immediate Wayland compositor pointer grab.
 * On Linux we use the attribute approach because startDragging() via GTK may
 * behave differently.
 *
 * On Windows/macOS, we use programmatic startDragging() via onMouseDown in
 * window-header.tsx. We intentionally do NOT set data-tauri-drag-region here
 * so Tauri's own drag.js does not intercept mousedown events — our handler
 * has full control over which elements drag and which don't.
 */
export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    const dragProps = isLinux() ? { "data-tauri-drag-region": true } : {};
    return { dragProps };
}
