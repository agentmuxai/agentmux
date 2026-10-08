// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai derived atom → plain function (reactive when called inside SolidJS tracking context)

import { createEffect, createRoot, on } from "solid-js";
import { atoms, getBlockComponentModel } from "@/app/store/global";
import { modalsModel } from "@/app/store/modalmodel";
import { caretInEditableOutsidePanes, focusedBlockId, userCaretInBlock } from "@/util/focusutil";
import { getLayoutModelForStaticTab } from "@/layout/index";

/**
 * How many animation frames `giveBlockFocus` keeps trying a pane whose input
 * isn't there yet (a view mounting fresh, a window tab still being revealed):
 * about a second. SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md R1.
 */
export const FOCUS_RETRY_FRAMES = 60;
/** A pointer press outside every pane this recently means the user moved focus on purpose. */
const DELIBERATE_PRESS_MS = 300;

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
        giveBlockFocus(lnode.data.blockId);
    }

    /**
     * The caret follows the selection: put it in the active tab's selected
     * pane (its active pane tab), unless the user has it somewhere on purpose.
     * Called whenever the selection changes for a reason other than the user
     * placing the caret: a pane or pane tab closing, a view swapping inside a
     * pane, a window tab becoming active, a modal closing over a pane that is
     * gone, and by the safety net when focus falls onto `<body>`.
     * `model`: the layout model whose selection changed; a background tab's
     * model is ignored. SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md R1.
     */
    ensureSelectionFocused(_reason: string, model?: unknown): void {
        if (modalsModel.hasOpenModals()) return;
        const active = getLayoutModelForStaticTab();
        if (model != null && model !== active) return;
        // A caret the user put in the tab-rename field, a search box outside
        // the panes, … stays there.
        if (caretInEditableOutsidePanes()) return;
        const blockId = active?.focusedNode?.()?.data?.blockId;
        if (blockId == null) return;
        giveBlockFocus(blockId);
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
        // A pane that finishes mounting after the user already clicked into
        // one of its inputs must not take the caret back.
        if (userCaretInBlock(blockId)) return;
        giveFocus();
    }
}

/**
 * Put the caret in `blockId`'s default focus target — the view's
 * `giveFocus()`, else the block's hidden dummy input — UNLESS the user
 * already put it in a text-entry control inside that block. The single
 * implementation behind every "focus this pane" path (`refocusNode()` above,
 * block-component-registry's `refocusNode(blockId)`, block.tsx's click
 * handler) so they can't drift apart again: the reducer-driven path added by
 * #3519 was missing the "already focused within" check the click handler
 * had, and ate the first click on any input in an unselected pane
 * (SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md).
 */
export function giveBlockFocus(blockId: string): void {
    cancelFocusRetry();
    if (userCaretInBlock(blockId)) return;
    if (focusBlockTarget(blockId)) return;
    // Nothing to type into yet: the block's dummy input keeps the keys in the
    // pane (its shortcuts work) instead of on <body>, and the target is tried
    // again on the next frames in case the view is still mounting.
    document.getElementById(`${blockId}-dummy-focus`)?.focus({ preventScroll: true });
    scheduleFocusRetry(blockId);
}

/**
 * The block's focus target, in order: the view's `giveFocus()`; else the first
 * visible element in the block marked `data-pane-focus` (a pane's main text
 * input, for views without a `giveFocus` of their own). True when one took it.
 */
function focusBlockTarget(blockId: string): boolean {
    if (getBlockComponentModel(blockId)?.viewModel?.giveFocus?.()) return true;
    const el = paneFocusElement(blockId);
    if (el == null) return false;
    el.focus({ preventScroll: true });
    return document.activeElement === el;
}

function paneFocusElement(blockId: string): HTMLElement | null {
    if (typeof document.querySelectorAll !== "function") return null;
    for (const el of document.querySelectorAll<HTMLElement>(`[data-blockid="${CSS.escape(blockId)}"] [data-pane-focus]`)) {
        const visible = typeof el.checkVisibility === "function" ? el.checkVisibility() : el.offsetParent != null;
        if (visible && !(el as HTMLInputElement).disabled) return el;
    }
    return null;
}

let focusRetry: number | null = null;

function cancelFocusRetry(): void {
    if (focusRetry != null && typeof cancelAnimationFrame === "function") cancelAnimationFrame(focusRetry);
    focusRetry = null;
}

/** Keep trying `blockId`'s target while it is still the selection and the caret is still parked. */
function scheduleFocusRetry(blockId: string): void {
    if (typeof requestAnimationFrame !== "function") return;
    let frames = FOCUS_RETRY_FRAMES;
    const tick = (): void => {
        focusRetry = null;
        if (--frames < 0) return;
        if (getLayoutModelForStaticTab()?.focusedNode?.()?.data?.blockId !== blockId) return;
        const active = document.activeElement;
        const parked = active == null || active === document.body || active.id === `${blockId}-dummy-focus`;
        if (!parked || modalsModel.hasOpenModals()) return;
        if (focusBlockTarget(blockId)) return;
        focusRetry = requestAnimationFrame(tick);
    };
    focusRetry = requestAnimationFrame(tick);
}

let followInstalled = false;

/**
 * Install once at startup. Two things no single close path can own:
 * - a window tab becoming active, however it happened (a click, a close that
 *   promoted a neighbour): its selected pane gets the caret, retried until the
 *   tab is revealed;
 * - the safety net: when focus leaves an element inside a pane and lands on
 *   <body> (the focused element was removed or hidden: a pane, a pane tab or a
 *   drawer closing), and the user didn't press somewhere outside the panes,
 *   the caret goes back to the selection. Logged, so a path that relies on
 *   the net can be found and wired properly.
 * SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md R1, R2.
 */
export function installFocusFollowsSelection(): void {
    if (followInstalled || typeof document === "undefined") return;
    followInstalled = true;
    let lastPressOutsideAt = Number.NEGATIVE_INFINITY;
    document.addEventListener(
        "pointerdown",
        (e) => {
            const t = e.target as Element | null;
            if (!t?.closest?.("[data-blockid]")) lastPressOutsideAt = performance.now();
        },
        true
    );
    document.addEventListener(
        "focusout",
        (e) => {
            const from = e.target as Element | null;
            if (e.relatedTarget != null || !from?.closest?.("[data-blockid]")) return;
            requestAnimationFrame(() => {
                const active = document.activeElement;
                if (active != null && active !== document.body) return;
                if (!document.hasFocus()) return; // the window lost focus: not an orphan
                if (performance.now() - lastPressOutsideAt < DELIBERATE_PRESS_MS) return;
                console.info("[focus] orphan: caret fell to <body>; returning it to the selection");
                focusManager.ensureSelectionFocused("orphan");
            });
        },
        true
    );
    createRoot(() => {
        createEffect(
            on(
                () => atoms.activeTabId(),
                () => requestAnimationFrame(() => focusManager.ensureSelectionFocused("tab-activated")),
                { defer: true }
            )
        );
    });
}

export const focusManager = new FocusManager();
