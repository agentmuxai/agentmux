// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Everything outside this element", by structure, and a way to switch it without one big restyle.
 *
 * A drag that must stay in one place (a text selection in a pane, a drag in a field) used to put an attribute on
 * the root that a `*` rule matched, so the whole document restyled at the first pointer move: 200 to 300 ms on a
 * window of long transcripts, all of it in that move. The document outside an element is the siblings of each of
 * its ancestors, whole subtrees, so the attribute goes on those few roots instead, nearest first, a few at a time
 * (`stageAttribute`): the part next to the drag is switched first and no frame is spent on all of it.
 */

/** The roots of everything outside `keep` and the chain of its ancestors: each ancestor's other children, nearest first. */
export function outsideRoots(keep: Element): Element[] {
    const roots: Element[] = [];
    for (let node: Element | null = keep; node?.parentElement && node !== node.ownerDocument.body; node = node.parentElement) {
        for (const sibling of node.parentElement.children) {
            if (sibling !== node && !/^(SCRIPT|STYLE|LINK|META)$/.test(sibling.tagName)) roots.push(sibling);
        }
    }
    return roots;
}

/** How long, in ms, a frame spends on staged changes: one root always goes, more while it is under this. */
export const STAGE_BUDGET_MS = 6;

// What each root is to have, by attribute, and the order they were asked in.
const wanted = new Map<Element, Map<string, boolean>>();
let frame: number | undefined;

function apply(el: Element, attr: string, on: boolean): void {
    if (on) el.setAttribute(attr, "");
    else el.removeAttribute(attr);
}

/** Does the changes that are due within the budget; the rest wait for the next frame. */
function run(): void {
    frame = undefined;
    const start = performance.now();
    for (const [el, attrs] of wanted) {
        for (const [attr, on] of attrs) if (el.isConnected) apply(el, attr, on);
        wanted.delete(el);
        if (performance.now() - start >= STAGE_BUDGET_MS) break;
    }
    if (wanted.size > 0 && typeof requestAnimationFrame === "function") frame = requestAnimationFrame(run);
}

/**
 * Sets (`on`) or removes `attr` on each of `roots`: the first within the frame's budget now, the rest on the frames
 * after. A later call for the same root and attribute replaces an earlier one that has not been done yet.
 */
export function stageAttribute(roots: readonly Element[], attr: string, on: boolean): void {
    for (const el of roots) {
        const attrs = wanted.get(el) ?? new Map<string, boolean>();
        attrs.set(attr, on);
        wanted.delete(el);
        wanted.set(el, attrs);
    }
    if (frame !== undefined && typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame);
    run();
}

/** Does every change still waiting, now (a test, or a page going away). */
export function flushStaged(): void {
    for (const [el, attrs] of wanted) for (const [attr, on] of attrs) if (el.isConnected) apply(el, attr, on);
    wanted.clear();
    if (frame !== undefined && typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame);
    frame = undefined;
}
