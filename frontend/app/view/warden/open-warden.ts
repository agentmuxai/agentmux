// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { movedSection, sectionKeyOf } from "@/app/block/pane-sections";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { WardenSection } from "./warden-model";

/** Opens Warden, or focuses the one already in this tab, on `section`. The Kanban is no longer one
 *  of Warden's sections: `"kanban"` opens the Kanban pane, where it moved (MOVED_SECTIONS). */
export function openWarden(section: WardenSection | "kanban"): Promise<void> {
    const moved = movedSection("warden", section);
    const view = moved?.view ?? "warden";
    const meta = moved ? (moved.section === undefined ? {} : { [sectionKeyOf(view)]: moved.section }) : { [sectionKeyOf(view)]: section };
    return openOrFocusPaneByView(view, { meta: { view, ...meta } }, Object.keys(meta).length > 0 ? meta : undefined);
}
