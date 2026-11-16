// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dragged tab itself in its landing place (PaneLanding.tsx). A dragged pane stands in the place its drop
 * would leave it and is laid out for that place. A tab that leaves other tabs behind has no pane of its own
 * until the release, and its place held a picture of it resized to the place (the operator, 2026-11-13: "in
 * dragging panes, the target pane content will keep their proper look, but dragging a pane tab will get a
 * distored look").
 *
 * So the place holds the tab as its pane will show it:
 * - its content is the tab's own, not a second copy: the element that holds the tab in its pane (its
 *   member, app/tab/pane-leaf-members.tsx) is moved into the place while the place is shown and back when it
 *   goes, as a maximized pane's content is moved into the overlay. Nothing is mounted twice and nothing is
 *   remounted;
 * - around it is the shell the tab had in its pane when the drag began (`captureTabShell`): the elements
 *   between the pane and the content, by their classes and inline styles, and a copy of the pane's header
 *   with only this tab in it. The header is a copy, so it is laid out for the place too, and does nothing.
 *
 * A tab whose content is a native surface (a browser's page) does not land this way: the page is drawn above
 * the DOM by the host. Its place keeps the picture, at the picture's own size.
 *
 * Only state and DOM, with no dependencies, so the pane's members and the layout can both use it.
 */

import { createSignal } from "solid-js";

/** One element between a pane's root and its content, top down. */
interface ShellLevel {
    className: string;
    style: string;
}

/** The shell a tab had in its pane when its drag began. */
export interface TabShell {
    blockId: string;
    /** From the pane's root down to the element that holds the content. Empty for a pane with no shell. */
    levels: ShellLevel[];
    /** The pane's header with only this tab, and the level it is a child of. */
    header?: { el: HTMLElement; level: number };
}

const HEADER_SELECTOR = '[data-role="block-header"]';
const SLOT_SELECTOR = ".pane-leaf-keepalive-slot";
const PILL_SELECTOR = ".pane-tab[data-tab-id]";
/** What a pill shows only while it is dragged, selected or on its way out. */
const PILL_DRAG_CLASSES = ["pane-tab--dragging", "pane-tab--gone", "pane-tab--selected", "pane-tab--drop-before", "pane-tab--drop-after"];

let shell: TabShell | undefined;
const members = new Map<string, number>();
const [mount, setMount] = createSignal<{ blockId: string; el: HTMLElement } | undefined>(undefined);

/** The pane's header with only tab `blockId` in it, in front: a copy that does nothing. */
function headerWithOnly(branch: Element, blockId: string): HTMLElement {
    const copy = branch.cloneNode(true) as HTMLElement;
    for (const el of [copy, ...Array.from(copy.querySelectorAll<HTMLElement>("[id]"))]) el.removeAttribute("id");
    for (const pill of Array.from(copy.querySelectorAll<HTMLElement>(PILL_SELECTOR))) {
        if (pill.dataset.tabId !== blockId) {
            pill.remove();
            continue;
        }
        pill.classList.remove(...PILL_DRAG_CLASSES);
        pill.classList.add("pane-tab--active");
    }
    return copy;
}

/**
 * Tab `blockId` of the pane `nodeId` is about to be dragged: keep the shell it has in the pane now, while it is
 * the tab in front and before the drag draws anything. Nothing is kept where the pane is not on screen.
 */
export function captureTabShell(blockId: string, nodeId: string): void {
    shell = undefined;
    if (typeof document === "undefined") return;
    const tile = document.getElementById(nodeId);
    // In the pane's leaf: the tile also holds a small copy of the pane for the drag's own picture.
    const leaf = tile?.querySelector(".tile-leaf");
    const block = Array.from(leaf?.querySelectorAll<HTMLElement>(".block[data-blockid]") ?? []).find(
        (el) => el.dataset.blockid === blockId
    );
    if (!tile || !block || !leaf) return;
    // The element that holds the content: the parent of the tab's slot, or of its block where it has no slot.
    const chain: HTMLElement[] = [];
    for (let el = (block.closest(SLOT_SELECTOR) ?? block).parentElement; el && el !== leaf && el !== tile; el = el.parentElement) {
        chain.unshift(el);
    }
    const kept: TabShell = { blockId, levels: chain.map((el) => ({ className: el.className, style: el.style.cssText })) };
    const header = leaf.querySelector(HEADER_SELECTOR);
    // A header inside the block goes with the block.
    if (header && !block.contains(header)) {
        // The lowest of them that holds the header, and its child the header is in.
        let level = chain.length - 1;
        while (level >= 0 && !chain[level].contains(header)) level--;
        const branch = level >= 0 && Array.from(chain[level].children).find((child) => child.contains(header));
        if (branch) kept.header = { el: headerWithOnly(branch, blockId), level };
    }
    shell = kept;
}

/** The shell kept for tab `blockId` when its drag began, or none. */
export const tabShellOf = (blockId: string): TabShell | undefined => (shell?.blockId === blockId ? shell : undefined);

/**
 * The shell built for the landing place: its root (none for a pane with no shell, whose content is all there
 * is) and the element the tab's content goes into.
 */
export function buildTabShell(kept: TabShell | undefined): { root: HTMLElement; content: HTMLElement } {
    const levels = kept?.levels ?? [];
    if (levels.length === 0) {
        const content = document.createElement("div");
        content.className = "pane-landing-content";
        return { root: content, content };
    }
    const els = levels.map((level) => {
        const el = document.createElement("div");
        el.className = level.className;
        el.style.cssText = level.style;
        return el;
    });
    els.forEach((el, i) => {
        if (i > 0) els[i - 1].appendChild(el);
    });
    if (kept?.header) els[kept.header.level].prepend(kept.header.el.cloneNode(true));
    return { root: els[0], content: els[els.length - 1] };
}

/** Tab `blockId`'s content can be moved (its member is mounted, and is not a native surface). Returns the undo. */
export function registerTabMember(blockId: string): () => void {
    // Counted: a tab that moves to another pane is mounted there before it is unmounted here.
    members.set(blockId, (members.get(blockId) ?? 0) + 1);
    return () => {
        const left = (members.get(blockId) ?? 1) - 1;
        if (left > 0) members.set(blockId, left);
        else members.delete(blockId);
    };
}

/** Whether tab `blockId`'s own content can stand in its landing place. */
export const tabMemberLands = (blockId: string): boolean => members.has(blockId);

/** The landing place of tab `blockId` is shown: its content goes into `el`. Returns the undo. */
export function showTabLanding(blockId: string, el: HTMLElement): () => void {
    setMount({ blockId, el });
    return () => {
        if (mount()?.el === el) setMount(undefined);
    };
}

/** Where tab `blockId`'s content stands while its landing place is shown, or nothing. Reactive. */
export function tabLandingMountOf(blockId: string): HTMLElement | undefined {
    const now = mount();
    return now?.blockId === blockId ? now.el : undefined;
}

/** Test hook. */
export function resetTabLandingForTests(): void {
    shell = undefined;
    members.clear();
    setMount(undefined);
}
