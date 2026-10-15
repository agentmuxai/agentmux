// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai Atom/Getter replaced by SolidJS signal accessors.

import { MOS } from "@/app/store/global";

/**
 * The tab's stored layout row, as an accessor. Read-only: the row changes
 * only through srv (layout intents and pane-tab commands, layoutIntents.ts).
 */
export function getLayoutStateAtomFromTab(tabAccessor: () => Tab): () => LayoutState | undefined {
    return () => {
        const tabData = tabAccessor();
        if (!tabData) return undefined;
        return MOS.getMuxObjectAtom<LayoutState>(MOS.makeORef("layout", tabData.layoutstate))();
    };
}
