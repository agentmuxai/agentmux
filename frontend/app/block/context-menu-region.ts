// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Context-menu regions: a way for one part of a pane to say "when a right-click
 * lands inside me, the pane menu looks like THIS".
 *
 * blockframe.tsx's onBodyContextMenu builds one menu for the whole pane body.
 * That is wrong for a sub-region with its own semantics — e.g. the agent pane's
 * shell drawer is a terminal, where Split/Replace With… (which act on the whole
 * agent pane) make no sense and Paste does. A region registers on its own
 * element; the body handler resolves the nearest one from the event target.
 *
 * A registry rather than a `data-` attribute because a region contributes items
 * with click closures over live state (the xterm instance), which an attribute
 * cannot carry. Rather than a ViewModel hook because that is per-view-model, and
 * a region is a slice of one pane.
 *
 * See docs/specs/SPEC_AGENT_SHELL_DRAWER_CONTEXT_MENU_PASTE_AND_REGIONS_2026_09_25.md.
 */

import type { PaneMenuSection } from "./pane-actions";

export interface ContextMenuRegion {
    /** Sections to drop from the pane menu for a right-click inside this region. */
    omit?: PaneMenuSection[];
    /**
     * Items rendered at the TOP of the menu, above the surviving pane sections.
     * Called at right-click time, not registration time, so it reads live state
     * (selection, agent lock, …).
     */
    items?: () => ContextMenuItem[];
}

const regions = new WeakMap<Element, ContextMenuRegion>();

/** Returns an unregister function — call it from onCleanup. */
export function registerContextMenuRegion(el: HTMLElement, region: ContextMenuRegion): () => void {
    regions.set(el, region);
    return () => {
        // Only remove our own entry: a re-registration on the same element must
        // not be undone by the earlier registration's late cleanup.
        if (regions.get(el) === region) regions.delete(el);
    };
}

/**
 * The nearest registered region at or above `target`, stopping at `boundary`
 * (exclusive — an element outside the pane's own frame is never consulted).
 * Nearest wins; regions never merge, so a region fully describes its own menu.
 */
export function resolveContextMenuRegion(
    target: EventTarget | null | undefined,
    boundary?: Element | null
): ContextMenuRegion | null {
    let el: Element | null = target instanceof Element ? target : null;
    while (el && el !== boundary) {
        const region = regions.get(el);
        if (region) return region;
        el = el.parentElement;
    }
    return null;
}
