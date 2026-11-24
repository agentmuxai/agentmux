// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { settledPaneRect } from "./settled-pane-rect";

interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

const COLLAPSED: Rect = { x: 0, y: 0, width: 0, height: 0 };

/**
 * The rect a native page is made at. A pane that can't be seen (a wake on hover, a tab loaded hidden) is made
 * collapsed, so its page never shows over the tab that is; the first sync on show lays it out. A pane that is
 * seen waits for its tile to hold still first (settled-pane-rect.ts), so the page lays out once, at its final
 * size, instead of narrow and then wider when the first rect after it lands. `measure` is null once the
 * placeholder is gone.
 */
export async function rectToCreatePageAt(visible: () => boolean, measure: () => Rect | null): Promise<Rect> {
    if (!visible()) return COLLAPSED;
    const rect = await settledPaneRect(() => measure() ?? COLLAPSED);
    return visible() && rect.width > 0 && rect.height > 0 ? rect : COLLAPSED;
}
