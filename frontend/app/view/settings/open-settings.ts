// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open Settings at a section, in this window. The pane open is the window's own
 * (`openOrFocusPaneByView`): the Settings pane already open here is shown and pointed at the
 * section, and only when there is none is one created, in this window's tab. Asking the server
 * (`PaneOpenCommand` with no tab) lets it pick the first workspace with an active tab, which can be
 * another window, and every click then adds a copy there.
 */

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

/** `board` (a board's slug) points a Kanban section at that board's stages (`settings:kanban-board`). */
export function openSettingsAt(section: string, board?: string): Promise<void> {
    const extra = board ? { "settings:kanban-board": board } : {};
    return openOrFocusPaneByView(
        "settings",
        { meta: { view: "settings", "settings:section": section, ...extra } },
        { "settings:section": section, ...extra }
    );
}
