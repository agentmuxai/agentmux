// Copyright 2026, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { isLinux } from "@/util/platformutil";

/**
 * Centralized window drag hook. Returns props to spread on a draggable element.
 *
 * On Linux, drag is handled natively by drag.rs (GTK motion detection) —
 * data-tauri-drag-region triggers an immediate Wayland compositor pointer grab
 * that swallows button clicks. So on Linux no drag attributes are set.
 *
 * On macOS/Windows, data-tauri-drag-region is handled at the WebView/OS level
 * (synchronous, before JS runs). JS startDragging() is intentionally NOT used —
 * it is async, conflicts with the native handler, and causes race-condition dead
 * spots. Element nesting handles exclusions: child elements with
 * data-tauri-drag-region="false" (TabBar, window controls) correctly block drag.
 */
export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    const dragProps = isLinux() ? {} : { "data-tauri-drag-region": true };
    return { dragProps };
}
