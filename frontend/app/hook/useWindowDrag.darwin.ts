// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS-specific window drag hook.
// data-drag-region is handled at the WebView/OS level (synchronous,
// before JS runs). Child elements with data-drag-region="false"
// correctly block drag.

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    return { dragProps: { "data-drag-region": true } };
}
