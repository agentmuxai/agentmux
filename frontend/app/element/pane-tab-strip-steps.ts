// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a strip reads of its surroundings to take its step (pane-tab-steps.ts): the room it has for its tabs and what
 * that gives each tab. In a pane's header the strip is one item of the header's scrolling range, so its room is the
 * range's width less everything else the range holds; anywhere else it is its parent's width. It is set on the strip
 * as a width limit (`--pane-tab-room`), so the tabs share the squeeze in plain CSS, and as the step (`data-step`), so the
 * labels go. Reads happen when the surroundings or the tab list change, never per tab per frame.
 */

import { createEffect, createSignal, on, onCleanup, onMount, type Accessor } from "solid-js";
import { minTabsRoom, stepFor, type PaneTabStep } from "./pane-tab-steps";

const HEADER_CONTENT = ".block-frame-header-content";
const HEADER_HOLDER = ".block-frame-default-header-tabstrip";
const ADD_BUTTON = ".pane-tab-strip-add-tip";

export interface StripSteps {
    step: Accessor<PaneTabStep>;
    /** The width, in real px, the strip may take, or undefined while it has not been measured. */
    room: Accessor<number | undefined>;
}

/** The width an element takes in its row, margins included, in real px. */
function outerWidth(el: Element): number {
    const style = getComputedStyle(el);
    return el.getBoundingClientRect().width + (parseFloat(style.marginLeft) || 0) + (parseFloat(style.marginRight) || 0);
}

/**
 * The strip's room in real px: the header range's width less what else the range holds, or the parent's width. Undefined
 * when the surroundings have no width (hidden).
 */
export function stripRoom(strip: HTMLElement): number | undefined {
    const holder = strip.closest<HTMLElement>(HEADER_HOLDER);
    const host = holder?.closest<HTMLElement>(HEADER_CONTENT);
    const range = host?.parentElement;
    if (holder && host && range) {
        let others = 0;
        let count = 0;
        for (const child of host.children) {
            if (child === holder || !(child instanceof HTMLElement) || child.offsetParent === null) continue;
            others += outerWidth(child);
            count++;
        }
        const gap = parseFloat(getComputedStyle(host).columnGap) || 0;
        const room = range.clientWidth - others - gap * count;
        return range.clientWidth > 0 ? room : undefined;
    }
    const parent = strip.parentElement;
    return parent && parent.clientWidth > 0 ? parent.clientWidth : undefined;
}

/** Whether a tab's label is cut short by the squeeze: the only time a strip needs a step. Reads every label once. */
function anyLabelCut(strip: HTMLElement): boolean {
    for (const label of strip.querySelectorAll<HTMLElement>(".pane-tab-label")) {
        if (label.scrollWidth > label.clientWidth + 1) return true;
    }
    return false;
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

    const measure = () => {
        frame = undefined;
        const strip = opts.strip();
        const count = opts.tabCount();
        const total = strip ? stripRoom(strip) : undefined;
        if (!strip || total === undefined || count < 1) return;
        const zoom = opts.zoom() || 1;
        const addWidth = strip.querySelector(ADD_BUTTON)?.getBoundingClientRect().width ?? 0;
        const tabsRoom = Math.max(0, total - addWidth);
        const share = tabsRoom / zoom / count;
        const previous = step();
        let next = stepFor(share, false, previous);
        // Labels are laid out at "full": if none is cut short the tabs fit as they are and nothing needs to give.
        if (next !== "full" && previous === "full" && !anyLabelCut(strip)) next = "full";
        if (next !== previous) setStep(next);
        // Never narrower than the tabs' own floors: past that the header scrolls, as it always did.
        const limit = Math.max(total, minTabsRoom(count) * zoom + addWidth);
        // A new limit changes how far the labels are cut: look once more when it has settled.
        if (room() === undefined || Math.abs((room() ?? 0) - limit) >= 0.5) {
            setRoom(limit);
            schedule();
        }
    };
    const schedule = () => {
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
            if (!host && strip.parentElement) next.add(strip.parentElement);
            for (const el of watched) if (!next.has(el)) observer?.unobserve(el);
            for (const el of next) if (!watched.has(el)) observer?.observe(el);
            watched = next;
            schedule();
        };
        watch();
        const host = strip.closest<HTMLElement>(HEADER_CONTENT);
        const mutations = host && typeof MutationObserver !== "undefined" ? new MutationObserver(watch) : undefined;
        if (host) mutations?.observe(host, { childList: true });
        onCleanup(() => {
            observer?.disconnect();
            mutations?.disconnect();
            if (frame !== undefined) cancelAnimationFrame(frame);
        });
    });
    createEffect(on([opts.tabCount, opts.zoom, opts.revision], schedule, { defer: true }));
    return { step, room };
}
