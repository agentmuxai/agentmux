// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai derived atom → plain function (reactive when called inside SolidJS tracking context)

import { getBlockComponentModel } from "@/app/store/global";
import { modalsModel } from "@/app/store/modalmodel";
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

    // Was a no-op (see SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md §2d /
    // SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §2b) — every within-tab
    // pane-selection path (click, arrow-key nav, Cmd+1..9, pane creation)
    // already dispatches through here via the layout tree's FocusNode/
    // InsertNode/MagnifyNodeToggle reducer cases; it just never DID
    // anything. Delegating to refocusNode() is what actually moves the
    // caret for all of them in one place.
    requestNodeFocus(): void {
        this.refocusNode();
    }

    getFocusType(): "node" {
        return "node";
    }

    refocusNode() {
        // Never steal the caret from an open modal (command palette,
        // settings, etc.) — see SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §5.
        if (modalsModel.hasOpenModals()) return;
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

    /**
     * Called from a pane's own onMount, once its focusable element (textarea,
     * CodeMirror view, xterm instance, …) is actually ready. Covers the case
     * `refocusNode()` cannot: a pane whose `giveFocus()` was attempted before
     * its view existed (creation, or switching into a tab whose pane hasn't
     * mounted yet) — see SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §3.
     *
     * Scoped to "is `blockId` the ACTIVE tab's currently-focused node" (not
     * just "focused within its own tab's tree") so a pane created or updated
     * in a background tab never steals focus from whatever the user is
     * actually looking at — see that spec's §5 constraint 1 and its analysis
     * of why `nodeModel.isFocused()` alone is not sufficient for this check.
     */
    claimFocusOnMount(blockId: string, giveFocus: () => boolean): void {
        if (modalsModel.hasOpenModals()) return;
        const layoutModel = getLayoutModelForStaticTab();
        const lnode = layoutModel?.focusedNode?.();
        if (lnode?.data?.blockId !== blockId) return;
        giveFocus();
    }
}

export const focusManager = new FocusManager();
