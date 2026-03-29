// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific window drag hook.
// Returns BOTH data-tauri-drag-region (for Tauri) and onMouseDown (for CEF).
// Tauri's native handler fires before JS and consumes the event, so onMouseDown
// never fires in Tauri. CEF ignores data-tauri-drag-region but handles onMouseDown.

import { detectHost, invokeCommand } from "@/app/platform/ipc";

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    return {
        dragProps: {
            "data-tauri-drag-region": true,
            onMouseDown: (e: MouseEvent) => {
                if (e.button !== 0) return;
                if (detectHost() !== "cef") return;
                invokeCommand("start_window_drag").catch(() => {});
            },
        },
    };
}
