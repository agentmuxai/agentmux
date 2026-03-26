// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai derived atom → plain function (reactive when called inside SolidJS tracking context)

import { getBlockComponentModel } from "@/app/store/global";
import { focusedBlockId } from "@/util/focusutil";
import { getLayoutModelForStaticTab } from "@/layout/index";

class FocusManager {
    /** Reactive accessor — returns the currently focused blockId (or null). */
    get blockFocusAtom(): () => string | null {
        return () => {
            const layoutModel = getLayoutModelForStaticTab();
            if (!layoutModel) return null;
            const lnode = layoutModel.focusedNode?.();
            return lnode?.data?.blockId ?? null;
        };
    }

    setBlockFocus(_force = false) {
        this.refocusNode();
    }

    nodeFocusWithin(): boolean {
        return focusedBlockId() != null;
    }

    requestNodeFocus(): void {
        // no-op
    }

    getFocusType(): "node" {
        return "node";
    }

    refocusNode() {
        const layoutModel = getLayoutModelForStaticTab();
        const lnode = layoutModel?.focusedNode?.();
        if (lnode == null || lnode.data?.blockId == null) return;
        layoutModel.focusNode(lnode.id);
        const blockId = lnode.data.blockId;
        const bcm = getBlockComponentModel(blockId);
        const ok = bcm?.viewModel?.giveFocus?.();
        if (!ok) {
            const inputElem = document.getElementById(`${blockId}-dummy-focus`);
            inputElem?.focus();
        }
    }
}

export const focusManager = new FocusManager();
