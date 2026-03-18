// Copyright 2026, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Centralized window drag hook. Returns props to spread on a draggable element.
 *
 * On Linux, drag is handled natively by drag.rs (GTK motion detection).
 * data-tauri-drag-region on Linux triggers a Wayland compositor pointer grab
 * that swallows button clicks — so no attributes are set.
 *
 * On Windows/macOS, window drag is handled programmatically via startDragging()
 * in window-header.tsx onMouseDown. No data-tauri-drag-region attributes are
 * needed — our handler has full control over which elements drag vs. click.
 */
export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    return { dragProps: {} };
}
