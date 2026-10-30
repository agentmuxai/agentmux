// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One delegated listener that shows the shared tooltip for every element with a native `title=`
// (the browser's own waits 0.5 to 1 s and cannot be sped up). The first time the pointer rests on
// one, its `title` moves to `data-title-tip` and tooltip-timing.ts takes over. The label stays for
// assistive technology: `aria-label` on an element with no text, else `aria-description`.
// Opt out with `data-native-title`; option, optgroup and iframe keep their native title.

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { isPrimaryButtonDown } from "@/app/util/pointer-drag-state";
import { claimOpen, HIDE_DELAY_MS, isWarm, releaseOpen, SHOW_DELAY_MS, type OpenTip } from "./tooltip-timing";

const CANDIDATE = "[title]";
const KEEP_NATIVE = "option, optgroup, iframe, [data-native-title]";
/** Set on an element whose `aria-label` or `aria-description` is this tooltip's text, so it can be taken back. */
const LABELLED = "data-title-tip-aria";

const PANEL_CLASS =
    "bg-modalbg border border-border rounded-md px-2 py-1 text-xs text-foreground shadow-xl z-[var(--z-tooltip)]";

/** The element whose label the pointer is over: the nearest one with a title, unless it keeps its native one. */
function candidateFor(target: EventTarget | null): HTMLElement | null {
    if (!(target instanceof Element)) return null;
    const el = target.closest<HTMLElement>(CANDIDATE);
    return el && !el.matches(KEEP_NATIVE) ? el : null;
}

function hasOwnText(el: HTMLElement): boolean {
    return (el.textContent ?? "").trim() !== "" || (el instanceof HTMLImageElement && el.alt !== "");
}

/**
 * Take `el`'s `title` for the shared tooltip while the pointer is on it, keeping its accessible
 * label. The attribute is left as `title=""` (which also keeps the browser's own tooltip away) so
 * that an application removing or changing the title meanwhile shows up as a difference.
 * Returns the label ("" for none). `giveTitleBack` undoes it.
 */
export function takeTitle(el: HTMLElement): string {
    const title = el.getAttribute("title");
    const held = el.dataset.titleTip;
    if (title === "" && held !== undefined) return held;
    if (title === null) {
        // Removed by the application while we held it, or never titled.
        if (held !== undefined) {
            delete el.dataset.titleTip;
            clearLabel(el);
        }
        return "";
    }
    clearLabel(el);
    if (title === "") {
        delete el.dataset.titleTip;
        return "";
    }
    el.setAttribute("title", "");
    el.dataset.titleTip = title;
    if (el.hasAttribute("aria-label") && !el.hasAttribute(LABELLED)) return title;
    if (el.hasAttribute("aria-labelledby") || hasOwnText(el)) {
        el.setAttribute("aria-description", title);
        el.setAttribute(LABELLED, "description");
    } else {
        el.setAttribute("aria-label", title);
        el.setAttribute(LABELLED, "label");
    }
    return title;
}

function clearLabel(el: HTMLElement): void {
    const kind = el.getAttribute(LABELLED);
    if (kind === "label") el.removeAttribute("aria-label");
    else if (kind === "description") el.removeAttribute("aria-description");
    el.removeAttribute(LABELLED);
}

/**
 * Put the title back when the pointer leaves, so the native title is there for assistive
 * technology. A title the application removed or changed meanwhile is left as it set it.
 */
export function giveTitleBack(el: HTMLElement): void {
    const text = el.dataset.titleTip;
    delete el.dataset.titleTip;
    clearLabel(el);
    if (text !== undefined && el.getAttribute("title") === "") el.setAttribute("title", text);
}

/**
 * Start showing the shared tooltip for titled elements in `doc`. Safe to call twice (the
 * first call wins); returns a function that stops it and removes the panel.
 */
let started: (() => void) | null = null;
export function initTitleTooltips(doc: Document = document): () => void {
    if (started) return started;
    const win = doc.defaultView ?? window;

    let panel: HTMLDivElement | null = null;
    let current: HTMLElement | null = null;
    let positioned = false;
    let visible = false;
    let showTimer: ReturnType<typeof setTimeout> | null = null;
    let hideTimer: ReturnType<typeof setTimeout> | null = null;
    let stopAutoUpdate: (() => void) | null = null;
    // After a click, a key or a wheel turn the tooltip goes but the pointer is still on the element:
    // its title stays taken (so the browser's slow tooltip does not follow) until the pointer leaves it.
    let held: HTMLElement | null = null;

    const self: OpenTip = { close: () => closeNow() };

    const clearTimers = () => {
        if (showTimer !== null) clearTimeout(showTimer);
        if (hideTimer !== null) clearTimeout(hideTimer);
        showTimer = hideTimer = null;
    };

    const paint = () => {
        if (panel) panel.style.opacity = visible && positioned ? "1" : "0";
    };

    const place = async () => {
        const reference = current;
        if (!reference || !panel) return;
        const pos = await computePosition(reference, panel, {
            placement: "top",
            middleware: [offset(10), flip(), shift({ padding: 12 })],
        });
        if (current !== reference || !panel) return;
        panel.style.left = `${pos.x}px`;
        panel.style.top = `${pos.y}px`;
        positioned = true;
        paint();
    };

    function closeNow(keepTitle = false): void {
        clearTimers();
        stopAutoUpdate?.();
        stopAutoUpdate = null;
        if (current) {
            releaseOpen(self);
            if (keepTitle) held = current;
            else giveTitleBack(current);
        }
        current = null;
        visible = false;
        positioned = false;
        panel?.remove();
    }

    const openFor = (el: HTMLElement, text: string) => {
        // Moving straight from one titled element to the next is a hand-over, whatever the shared state says.
        const handOver = current !== null;
        if (current && current !== el) giveTitleBack(current);
        clearTimers();
        stopAutoUpdate?.();
        claimOpen(self);
        current = el;
        visible = false;
        positioned = false;
        if (!panel) {
            panel = doc.createElement("div");
            panel.className = PANEL_CLASS;
            panel.setAttribute("data-pane-overlay", "");
            panel.setAttribute("role", "tooltip");
            panel.style.cssText =
                "position:absolute;left:0;top:0;opacity:0;pointer-events:none;max-width:24rem;white-space:pre-line;" +
                "transition:opacity var(--duration-instant) var(--ease-out)";
        }
        panel.textContent = text;
        panel.style.opacity = "0";
        doc.body.append(panel);
        stopAutoUpdate = autoUpdate(el, panel, () => void place());
        void place();
        const delay = handOver || isWarm(self) ? 0 : SHOW_DELAY_MS;
        const show = () => {
            visible = true;
            paint();
        };
        if (delay === 0) show();
        else showTimer = setTimeout(show, delay);
    };

    const scheduleClose = () => {
        if (!current || hideTimer !== null) return;
        hideTimer = setTimeout(closeNow, HIDE_DELAY_MS);
    };

    const letGo = () => {
        if (held) giveTitleBack(held);
        held = null;
    };

    const onOver = (e: Event) => {
        if (held) {
            // Still on the element that was clicked: it stays quiet until the pointer leaves it.
            if (e.target instanceof Node && held.contains(e.target)) return;
            letGo();
        }
        // The title is off the element while the pointer is on it, so inside it the element itself is the candidate.
        const el = candidateFor(e.target) ?? (current && e.target instanceof Node && current.contains(e.target) ? current : null);
        if (!el) return scheduleClose();
        if (el === current) {
            if (hideTimer !== null) {
                clearTimeout(hideTimer);
                hideTimer = null;
            }
            // A reactive label may have changed while it is open.
            const text = takeTitle(el);
            if (!text) return closeNow();
            if (panel && panel.textContent !== text) panel.textContent = text;
            return;
        }
        if (isPrimaryButtonDown()) return;
        const text = takeTitle(el);
        if (!text) return scheduleClose();
        openFor(el, text);
    };

    // A click, a key or a wheel turn means the pointer is not resting any more.
    const hideNow = () => closeNow(true);
    const hideAll = () => {
        closeNow();
        letGo();
    };

    doc.addEventListener("pointerover", onOver, true);
    doc.addEventListener("pointerdown", hideNow, true);
    doc.addEventListener("keydown", hideNow, true);
    doc.addEventListener("wheel", hideNow, { capture: true, passive: true });
    doc.documentElement.addEventListener("pointerleave", scheduleClose);
    win.addEventListener("blur", hideAll);

    const stop = () => {
        hideAll();
        doc.removeEventListener("pointerover", onOver, true);
        doc.removeEventListener("pointerdown", hideNow, true);
        doc.removeEventListener("keydown", hideNow, true);
        doc.removeEventListener("wheel", hideNow, true);
        doc.documentElement.removeEventListener("pointerleave", scheduleClose);
        win.removeEventListener("blur", hideAll);
        started = null;
    };
    started = stop;
    return stop;
}
