// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai Atom/Getter replaced by SolidJS signal accessors.

import { MOS } from "@/app/store/global";
import { ObjectService } from "@/app/store/services";
import type { SignalAtom } from "@/util/util";

/** srv's answer to a layout push it refused: built on `layoutrevision` or older. */
export type LayoutPushResult = { stale?: boolean; layoutrevision?: number };

/** Returns a SignalAtom for the LayoutState belonging to the given tab. */
export function getLayoutStateAtomFromTab(tabAccessor: () => Tab): SignalAtom<LayoutState> {
    function getOref(): string | null {
        const tabData = tabAccessor();
        if (!tabData) return null;
        return MOS.makeORef("layout", tabData.layoutstate);
    }

    const atom = () => {
        const oref = getOref();
        if (!oref) return undefined;
        return MOS.getMuxObjectAtom<LayoutState>(oref)();
    };

    (atom as any)._set = (value: LayoutState | ((prev: LayoutState) => LayoutState)) => {
        const oref = getOref();
        if (!oref) return;
        const wovAtom = MOS.getMuxObjectAtom<LayoutState>(oref);
        const nextValue = typeof value === "function" ? (value as (prev: LayoutState) => LayoutState)(wovAtom()) : value;
        MOS.setObjectValue(nextValue, true);
    };
    // The same write, resolving to the push's answer: `{ stale, layoutrevision }`
    // when srv refused a push built on a stale layout revision.
    (atom as any)._push = (value: LayoutState): Promise<LayoutPushResult | null> => {
        MOS.setObjectValue(value, false);
        try {
            return Promise.resolve(ObjectService.UpdateObject(value, false) as unknown as LayoutPushResult | null);
        } catch (e) {
            return Promise.reject(e);
        }
    };

    return atom as unknown as SignalAtom<LayoutState>;
}
