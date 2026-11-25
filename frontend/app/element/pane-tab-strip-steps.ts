// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a strip reads of its surroundings to take its step (pane-tab-steps.ts): the room it has for its tabs and what
 * that gives each tab. In a pane's header the strip is one item of the header's scrolling range, so its room is the
 * range's width less everything else the range holds; anywhere else it is its parent's width. It is set on the strip
 * as a width limit (`--pane-tab-room`, written to the strip's style here), so the tabs share the squeeze in plain CSS, and as the step (`data-step`), so the
 * labels go. Reads happen when the surroundings or the tab list change, never per tab per frame.
 */

import { createEffect, createSignal, on, onCleanup, onMount, type Accessor } from "solid-js";
import { afterSplitterDrag, splitterDragActive } from "@/app/block/pane-body-hold";
import {
    ACTIVE_FLOOR_PX,
    ACTIVE_NO_ICON_FLOOR_PX,
    activeFloor,
    activeWant,
    minTabsRoom,
    othersShare,
    RESTING_FLOOR_PX,
    RESTING_NO_ICON_FLOOR_PX,
    stepFor,
    type PaneTabStep,
} from "./pane-tab-steps";

const HEADER_CONTENT = ".block-frame-header-content";
const HEADER_HOLDER = ".block-frame-default-header-tabstrip";
const ADD_BUTTON = ".pane-tab-strip-add-tip";

export interface StripSteps {
    step: Accessor<PaneTabStep>;
    /** The width, in real px, the strip may take, or undefined while it has not been measured. */
    room: Accessor<number | undefined>;
}

/**
 * How much bigger than its layout width an element is drawn: the chrome zoom, which the Editor's document bar is drawn at,
 * scales what getBoundingClientRect reports but not clientWidth. 1 when it can't be told (no layout).
 */
function layoutScale(el: HTMLElement): number {
    const layout = el.clientWidth;
    const drawn = el.getBoundingClientRect().width;
    return layout > 0 && drawn > 0 ? drawn / layout : 1;
}

/** The width an element takes in its row, margins included, in the row's own (layout) px: `scale` is how much bigger it is drawn. */
function outerWidth(el: Element, scale: number): number {
    const style = getComputedStyle(el);
    return el.getBoundingClientRect().width / scale + (parseFloat(style.marginLeft) || 0) + (parseFloat(style.marginRight) || 0);
}

/**
 * What a growing item of the header needs, not the width it has grown to: its own contents. An item that grows into
 * free room (the header's inline elements, the spacer) would otherwise count the room the strip just left as its own.
 */
function contentWidth(el: HTMLElement, scale: number): number {
    let width = 0;
    let count = 0;
    for (const child of el.children) {
        if (!(child instanceof HTMLElement) || child.offsetParent === null) continue;
        width += outerWidth(child, scale);
        count++;
    }
    return width + (parseFloat(getComputedStyle(el).columnGap) || 0) * Math.max(0, count - 1);
}

/** What the visible children of `row` other than `except` take of it: their widths and the gaps between them and `except`. */
function othersIn(row: HTMLElement, except: Element, scale: number): number {
    let others = 0;
    let count = 0;
    for (const child of row.children) {
        if (child === except || !(child instanceof HTMLElement) || child.offsetParent === null) continue;
        others += (parseFloat(getComputedStyle(child).flexGrow) || 0) > 0 ? contentWidth(child, scale) : outerWidth(child, scale);
        count++;
    }
    return others + (parseFloat(getComputedStyle(row).columnGap) || 0) * count;
}

/**
 * The strip's room in real px: the header range's width less what else the range holds, or, outside a header, the parent's
 * width less what else is in the parent's row (the Editor's file tree toggle before its tabs). Undefined when the
 * surroundings have no width (hidden).
 */
export function stripRoom(strip: HTMLElement): number | undefined {
    const holder = strip.closest<HTMLElement>(HEADER_HOLDER);
    const host = holder?.closest<HTMLElement>(HEADER_CONTENT);
    const range = host?.parentElement;
    if (holder && host && range) {
        return range.clientWidth > 0 ? range.clientWidth - othersIn(host, holder, layoutScale(range)) : undefined;
    }
    const parent = strip.parentElement;
    return parent && parent.clientWidth > 0 ? parent.clientWidth - othersIn(parent, strip, layoutScale(parent)) : undefined;
}

/** The padding a pill keeps at its right: its own `padding-right` and the border (PaneTabStrip.scss `.pane-tab`), in unzoomed px. */
const PILL_RIGHT_PX = 5;

/**
 * What the selected tab needs, in unzoomed px: `fixed`, all of the pill that is not its title (icon, the pane's own
 * buttons, close button, the gaps and padding), and `label`, how long its title is. Read from where the pill's items
 * stand, not from the pill's width, so it is the same whether the pill is squeezed or not. Null with no selected tab or
 * no room to measure.
 */
export function activeTabNeeds(strip: HTMLElement, zoom: number): { fixed: number; label: number } | null {
    const pill = strip.querySelector<HTMLElement>(".pane-tab--active");
    const label = pill?.querySelector<HTMLElement>(".pane-tab-label");
    if (!pill || !label) return null;
    const items = [...pill.children].filter(
        (el): el is HTMLElement => el instanceof HTMLElement && el.offsetParent !== null && getComputedStyle(el).position !== "absolute"
    );
    const pillBox = pill.getBoundingClientRect();
    if (items.length === 0 || pillBox.width === 0) return null;
    const first = items[0].getBoundingClientRect();
    const last = items[items.length - 1].getBoundingClientRect();
    const labelBox = label.getBoundingClientRect();
    // The items' span, from the first to the last, less the title between them: the same squeezed or not.
    const inner = last.right - first.left - labelBox.width;
    const pad = Math.max(0, first.left - pillBox.left);
    // The title's own length: its box as laid out, scaled as its content is (a title that is not cut is its own width).
    const scale = label.clientWidth > 0 ? labelBox.width / label.clientWidth : 1;
    return { fixed: (inner + pad) / zoom + PILL_RIGHT_PX, label: (label.scrollWidth * scale) / zoom };
}

/** Whether a tab's label is cut short by the squeeze: the only time a strip needs a step. Reads every label once. */
function anyLabelCut(strip: HTMLElement): boolean {
    for (const label of strip.querySelectorAll<HTMLElement>(".pane-tab-label")) {
        if (label.scrollWidth > label.clientWidth + 1) return true;
    }
    return false;
}

/**
 * The room the strip is given, never narrower than the tabs' own floors: past that the header scrolls, as it always did.
 * A tab with no icon keeps a wider floor (a few letters of its label), as its style says; the selected tab's own floor is
 * its measured one, at least that.
 */
function roomLimit(strip: HTMLElement, a: { total: number; count: number; zoom: number; addWidth: number; floor: number | undefined }): number {
    const noIcon = strip.querySelector(".pane-tab.doc-tab--no-icon") !== null;
    const resting = noIcon ? RESTING_NO_ICON_FLOOR_PX : RESTING_FLOOR_PX;
    const activeLeast = noIcon ? ACTIVE_NO_ICON_FLOOR_PX : ACTIVE_FLOOR_PX;
    return Math.max(a.total, minTabsRoom(a.count, resting, Math.max(a.floor ?? activeLeast, activeLeast)) * a.zoom + a.addWidth);
}

/**
 * Keeps `strip`'s step and room current: called once, in the strip's own component. `tabCount` and `zoom` are what
 * the strip draws now; `revision` changes when a label does (the tab list, a rename), so a strip whose labels got longer
 * looks again.
 */
export function createStripSteps(opts: {
    strip: () => HTMLElement | undefined;
    tabCount: Accessor<number>;
    zoom: Accessor<number>;
    revision: Accessor<unknown>;
}): StripSteps {
    const [step, setStep] = createSignal<PaneTabStep>("full");
    const [room, setRoom] = createSignal<number | undefined>(undefined);
    let frame: number | undefined;
    /** The floor last written to the strip. */
    let lastFloor: number | undefined;

    const measure = () => {
        if (frame !== undefined) cancelAnimationFrame(frame);
        frame = undefined;
        const strip = opts.strip();
        const count = opts.tabCount();
        const total = strip ? stripRoom(strip) : undefined;
        if (!strip || total === undefined || count < 1) return;
        const zoom = opts.zoom() || 1;
        const addWidth = strip.querySelector(ADD_BUTTON)?.getBoundingClientRect().width ?? 0;
        const tabsRoom = Math.max(0, total - addWidth);
        // The selected tab has its room first (icon, its own buttons, close button and a readable start of its title); the
        // other tabs give up width before it does, so the step is the one their share asks for.
        const needs = activeTabNeeds(strip, zoom);
        const share = needs ? othersShare(tabsRoom / zoom, count, activeWant(needs.fixed, needs.label)) : tabsRoom / zoom / count;
        const floor = needs ? activeFloor(needs.fixed, needs.label) : undefined;
        if (floor !== undefined && Math.abs(floor - (lastFloor ?? -1)) >= 0.5) {
            lastFloor = floor;
            strip.style.setProperty("--pane-tab-active-floor", `${floor}px`);
            // The selected tab keeps another width now: look once more when it has settled.
            schedule();
        } else if (floor === undefined && lastFloor !== undefined) {
            lastFloor = undefined;
            strip.style.removeProperty("--pane-tab-active-floor");
        }
        const previous = step();
        let next = stepFor(share, false, previous);
        // Labels are laid out at "full": if none is cut short the tabs fit as they are and nothing needs to give.
        if (next !== "full" && previous === "full" && !anyLabelCut(strip)) next = "full";
        // A rename in progress holds the step: the input must not change under the typing.
        if (strip.querySelector(".pane-tab-rename-input")) next = previous;
        if (next !== previous) setStep(next);
        const limit = roomLimit(strip, { total, count, zoom, addWidth, floor });
        // A new limit changes how far the labels are cut: look once more when it has settled.
        if (room() === undefined || Math.abs((room() ?? 0) - limit) >= 0.5) {
            setRoom(limit);
            strip.style.setProperty("--pane-tab-room", `${limit}px`);
            schedule();
        }
    };
    const schedule = () => {
        // Not while a pane border is dragged: the strip's room changes every frame, and each look reads the layout. One look at the end.
        if (splitterDragActive()) return afterSplitterDrag(schedule);
        if (frame === undefined) frame = requestAnimationFrame(measure);
    };

    onMount(() => {
        const strip = opts.strip();
        if (!strip) return;
        const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(schedule);
        let watched = new Set<Element>();
        // The range, and the few other things in it (not the tabs), whose widths decide the strip's room.
        const watch = () => {
            const holder = strip.closest<HTMLElement>(HEADER_HOLDER);
            const host = holder?.closest<HTMLElement>(HEADER_CONTENT);
            const next = new Set<Element>();
            if (host?.parentElement) next.add(host.parentElement);
            if (host) for (const child of host.children) if (child !== holder) next.add(child);
            if (!host && strip.parentElement) {
                next.add(strip.parentElement);
                for (const child of strip.parentElement.children) if (child !== strip) next.add(child);
            }
            for (const el of watched) if (!next.has(el)) observer?.unobserve(el);
            for (const el of next) if (!watched.has(el)) observer?.observe(el);
            watched = next;
            schedule();
        };
        watch();
        // At once, not a frame later: the limit is on the strip before anything else first measures it.
        measure();
        // The row the strip shares with other items: an item that comes or goes (the Editor's tree toggle) changes its room.
        const row = strip.closest<HTMLElement>(HEADER_CONTENT) ?? strip.parentElement;
        const mutations = row && typeof MutationObserver !== "undefined" ? new MutationObserver(watch) : undefined;
        if (row) mutations?.observe(row, { childList: true });
        // Another tab selected, or the selected tab's own buttons come or go: what it needs has changed.
        const tabs = typeof MutationObserver === "undefined" ? undefined : new MutationObserver(schedule);
        tabs?.observe(strip, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
        onCleanup(() => {
            observer?.disconnect();
            mutations?.disconnect();
            tabs?.disconnect();
            if (frame !== undefined) cancelAnimationFrame(frame);
        });
    });
    createEffect(on([opts.tabCount, opts.zoom, opts.revision], schedule, { defer: true }));
    return { step, room };
}
