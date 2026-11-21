// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The block ids of the panes the page has mounted in a tab's container, for the pane watchdog (pane-watchdog.ts). A
 * pane is there when its frame is, and also when its own error boundary has caught a crash and shows its fallback
 * (which has its own Reload pane and Close controls): that pane is showing, and is not what the offer is for. A drag
 * preview's frame is not a pane.
 */
export const MOUNTED_PANE_SELECTOR = ".block-frame-default[data-blockid]:not(.block-preview), .block-error-fallback[data-blockid]";

export function mountedPaneIds(tabEl: Element | null | undefined): Set<string> {
    const ids = new Set<string>();
    tabEl?.querySelectorAll(MOUNTED_PANE_SELECTOR).forEach((el) => {
        const id = el.getAttribute("data-blockid");
        if (id) ids.add(id);
    });
    return ids;
}
