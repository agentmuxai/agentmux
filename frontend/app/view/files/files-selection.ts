// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's selection model, kept separate from focus as the ARIA
 * treegrid pattern asks: focus is one row (the keyboard's position), the
 * selection is a set of names, and the anchor is where a Shift range starts.
 * Rows are identified by name, so a re-sort or a re-list keeps both.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.3.
 */

export interface Selection {
    /** Selected names. */
    names: ReadonlySet<string>;
    /** The focused row's name, or null before anything is focused. */
    focus: string | null;
    /** Where a Shift-extended range starts. */
    anchor: string | null;
}

export const EMPTY_SELECTION: Selection = { names: new Set(), focus: null, anchor: null };

/** The names from `a` to `b`, inclusive, in the order they're shown. */
function rangeOf(order: readonly string[], a: string, b: string): string[] {
    const i = order.indexOf(a);
    const j = order.indexOf(b);
    if (i < 0 || j < 0) return [b];
    return order.slice(Math.min(i, j), Math.max(i, j) + 1);
}

/** A click on `name`: plain selects only it, Ctrl/Cmd toggles it, Shift
 *  selects the range from the anchor (adding to the selection with Ctrl). */
export function clickRow(
    sel: Selection,
    order: readonly string[],
    name: string,
    mods: { toggle: boolean; range: boolean }
): Selection {
    if (mods.range && sel.anchor != null) {
        const range = rangeOf(order, sel.anchor, name);
        const names = mods.toggle ? new Set([...sel.names, ...range]) : new Set(range);
        return { names, focus: name, anchor: sel.anchor };
    }
    if (mods.toggle) {
        const names = new Set(sel.names);
        if (names.has(name)) names.delete(name);
        else names.add(name);
        return { names, focus: name, anchor: name };
    }
    return { names: new Set([name]), focus: name, anchor: name };
}

/**
 * Moving focus by `delta` rows (or to an absolute index with `to`). Without
 * Shift the selection follows focus; with Shift it becomes the range from the
 * anchor; with Ctrl only focus moves (Space then toggles).
 */
export function moveFocus(
    sel: Selection,
    order: readonly string[],
    move: { delta?: number; to?: number },
    mods: { extend: boolean; focusOnly: boolean }
): Selection {
    if (order.length === 0) return sel;
    const cur = sel.focus != null ? order.indexOf(sel.focus) : -1;
    let next = move.to ?? (cur < 0 ? (move.delta ?? 0) > 0 ? 0 : order.length - 1 : cur + (move.delta ?? 0));
    next = Math.max(0, Math.min(order.length - 1, next));
    const name = order[next];
    if (mods.focusOnly) return { ...sel, focus: name };
    if (mods.extend) {
        const anchor = sel.anchor ?? sel.focus ?? name;
        return { names: new Set(rangeOf(order, anchor, name)), focus: name, anchor };
    }
    return { names: new Set([name]), focus: name, anchor: name };
}

/** Space: toggle the focused row. */
export function toggleFocused(sel: Selection): Selection {
    if (sel.focus == null) return sel;
    const names = new Set(sel.names);
    if (names.has(sel.focus)) names.delete(sel.focus);
    else names.add(sel.focus);
    return { ...sel, names, anchor: sel.focus };
}

export function selectAll(sel: Selection, order: readonly string[]): Selection {
    return { names: new Set(order), focus: sel.focus ?? order[0] ?? null, anchor: sel.anchor ?? order[0] ?? null };
}

/** Drop names no longer listed (a re-list after a change on disk). Focus moves
 *  to the nearest surviving row in the old order. */
export function pruneSelection(sel: Selection, oldOrder: readonly string[], newOrder: readonly string[]): Selection {
    const present = new Set(newOrder);
    const names = new Set([...sel.names].filter((n) => present.has(n)));
    let focus = sel.focus;
    if (focus != null && !present.has(focus)) {
        const i = oldOrder.indexOf(focus);
        focus = null;
        for (let d = 1; i >= 0 && d <= oldOrder.length; d++) {
            const after = oldOrder[i + d];
            const before = oldOrder[i - d];
            if (after != null && present.has(after)) {
                focus = after;
                break;
            }
            if (before != null && present.has(before)) {
                focus = before;
                break;
            }
        }
    }
    const anchor = sel.anchor != null && present.has(sel.anchor) ? sel.anchor : focus;
    return { names, focus, anchor };
}
