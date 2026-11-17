// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's colours on a panel it opens outside itself (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §7.4).
 *
 * A menu, a popover, a tooltip or a hover panel is mounted on `document.body`, outside the pane's element, so the
 * properties a pane with a backdrop carries do not reach it by inheritance: it drew in the window's theme over a pane
 * that has its own. `carryPaneScope` puts the pane's properties on the panel's root, for the overlay layer
 * (`overlayScopeVars`), and keeps them current while the panel is open.
 *
 * Which pane: the one the panel was opened from. Every mount point already has an element to ask (its anchor, the
 * element under the pointer, the row a panel is for), and `blockOf` finds the pane that element is in. A panel that
 * carries a scope says which pane it is for, so a panel opened from it (a submenu, a tooltip on a menu row) finds
 * the same pane. A panel opened from outside any pane, or from a pane with no backdrop, is left exactly as it was.
 */

import { createEffect, createRoot } from "solid-js";
import { getPaneScope, overlayScopeVars } from "./scope";

/** On a scoped panel: the block whose scope it carries. */
export const OVERLAY_BLOCK_ATTR = "data-backdrop-block";

const OWNER = `[data-blockid], [${OVERLAY_BLOCK_ATTR}]`;

/** What says which pane a panel belongs to: an element in the pane (or in a panel the pane opened), or a block id. */
export type ScopeSource = Element | DOMRect | string | null | undefined;

/** The block a source belongs to; undefined for a point, a rect or an element outside every pane. */
export function blockOf(source: ScopeSource): string | undefined {
    if (!source) return undefined;
    if (typeof source === "string") return source;
    if (!(source instanceof Element)) return undefined;
    const owner = source.closest<HTMLElement>(OWNER);
    return owner?.getAttribute(OVERLAY_BLOCK_ATTR) || owner?.dataset.blockid || undefined;
}

/**
 * Keep `el`, a panel's root, carrying the scope of the pane `source` is in: its properties for the overlay layer,
 * `data-backdrop` (so the aliases are resolved again on it, `_aliases.scss`) and the block's id. It follows the pane:
 * a backdrop changed or removed while the panel is open changes or clears the panel's too.
 *
 * Returns the function that stops following and takes everything off again, or undefined when `source` is in no
 * pane (nothing was done; a caller whose source can arrive later may ask again).
 */
export function carryPaneScope(el: HTMLElement, source: ScopeSource): (() => void) | undefined {
    const blockId = blockOf(source);
    if (!blockId) return undefined;
    const scope = getPaneScope(blockId);
    let set: string[] = [];
    const clear = () => {
        for (const name of set) el.style.removeProperty(name);
        set = [];
        el.removeAttribute("data-backdrop");
        el.removeAttribute(OVERLAY_BLOCK_ATTR);
    };
    return createRoot((dispose) => {
        createEffect(() => {
            const vars = scope();
            clear();
            if (!vars) return;
            const overlay = overlayScopeVars(vars);
            for (const [name, value] of Object.entries(overlay)) el.style.setProperty(name, value);
            set = Object.keys(overlay);
            el.setAttribute("data-backdrop", "");
            el.setAttribute(OVERLAY_BLOCK_ATTR, blockId);
        });
        return () => {
            dispose();
            clear();
        };
    });
}
