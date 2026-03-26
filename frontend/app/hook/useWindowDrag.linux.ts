// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Linux-specific window drag hook.
// On Linux, drag is handled natively by drag.rs (GTK motion detection).
// data-tauri-drag-region triggers an immediate Wayland compositor pointer
// grab that swallows button clicks, so no drag attributes are set.

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    return { dragProps: {} };
}
