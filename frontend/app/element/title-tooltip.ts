// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One delegated listener that shows the shared tooltip for every element with a native `title=`
// (the browser's own waits 0.5 to 1 s and cannot be sped up). The first time the pointer rests on
// one, its `title` moves to `data-title-tip` and tooltip-timing.ts takes over. The label stays for
// assistive technology: `aria-label` on an element with no text, else `aria-description`.
// Opt out with `data-native-title`; option, optgroup and iframe keep their native title.
// An element with `data-hover-tip` (tip.ts) gets the same tooltip with no title to move: its label and
// its accessible name or description are already on it. (The status bar's own `data-tip` is a different
// tooltip, StatusBarTip.tsx, and is left alone.)

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { isPrimaryButtonDown } from "@/app/util/pointer-drag-state";
import { claimOpen, HIDE_DELAY_MS, isWarm, releaseOpen, SHOW_DELAY_MS, type OpenTip } from "./tooltip-timing";
import "./anchored-popover.scss";

const CANDIDATE = "[title], [data-hover-tip]";
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
    if (title === null && held === undefined && el.hasAttribute("data-hover-tip")) return el.dataset.hoverTip ?? "";
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

/** The shell AnchoredPopover is made of (placed in real pixels, never zoomed) around the body (chrome zoom, the fade). */
function createPanel(doc: Document): { shell: HTMLDivElement; panel: HTMLDivElement } {
    const panel = doc.createElement("div");
    panel.className = `${PANEL_CLASS} anchored-popover-body anchored-popover-body--chrome-zoom`;
    panel.setAttribute("role", "tooltip");
    panel.style.cssText =
        "opacity:0;pointer-events:none;max-width:24rem;white-space:pre-line;" +
        "transition:opacity var(--duration-instant) var(--ease-out)";
    const shell = doc.createElement("div");
    shell.className = "anchored-popover anchored-popover--tooltip";
    shell.setAttribute("data-pane-overlay", "");
    shell.style.cssText = "left:0;top:0;pointer-events:none";
    shell.append(panel);
    return { shell, panel };
}

/** What the tooltip is doing: its panel, the element it is for, its timers and the element whose title stays taken. */
interface TipState {
    doc: Document;
    // The same shell and body AnchoredPopover is made of (anchored-popover.tsx), built by hand because this tooltip is
    // not a component: the shell is placed in real pixels and never zoomed, the body inside it carries chrome zoom.
    shell: HTMLDivElement | null;
    panel: HTMLDivElement | null;
    current: HTMLElement | null;
    positioned: boolean;
    visible: boolean;
    showTimer: ReturnType<typeof setTimeout> | null;
    hideTimer: ReturnType<typeof setTimeout> | null;
    stopAutoUpdate: (() => void) | null;
    // After a click, a key or a wheel turn the tooltip goes but the pointer is still on the element:
    // its title stays taken (so the browser's slow tooltip does not follow) until the pointer leaves it.
    held: HTMLElement | null;
    self: OpenTip;
}

function createTipState(doc: Document): TipState {
    const t: TipState = {
        doc,
        shell: null,
        panel: null,
        current: null,
        positioned: false,
        visible: false,
        showTimer: null,
        hideTimer: null,
        stopAutoUpdate: null,
        held: null,
        self: { close: () => closeNow(t) },
    };
    return t;
}

function clearTimers(t: TipState): void {
    if (t.showTimer !== null) clearTimeout(t.showTimer);
    if (t.hideTimer !== null) clearTimeout(t.hideTimer);
    t.showTimer = t.hideTimer = null;
}

function paint(t: TipState): void {
    if (t.panel) t.panel.style.opacity = t.visible && t.positioned ? "1" : "0";
}

async function place(t: TipState): Promise<void> {
    const reference = t.current;
    if (!reference || !t.shell) return;
    const pos = await computePosition(reference, t.shell, {
        strategy: "fixed",
        placement: "top",
        middleware: [offset(10), flip(), shift({ padding: 12 })],
    });
    if (t.current !== reference || !t.shell) return;
    t.shell.style.left = `${pos.x}px`;
    t.shell.style.top = `${pos.y}px`;
    t.positioned = true;
    paint(t);
}

function closeNow(t: TipState, keepTitle = false): void {
    clearTimers(t);
    t.stopAutoUpdate?.();
    t.stopAutoUpdate = null;
    if (t.current) {
        releaseOpen(t.self);
        if (keepTitle) t.held = t.current;
        else giveTitleBack(t.current);
    }
    t.current = null;
    t.visible = false;
    t.positioned = false;
    t.shell?.remove();
}

function openFor(t: TipState, el: HTMLElement, text: string): void {
    // Moving straight from one titled element to the next is a hand-over, whatever the shared state says.
    const handOver = t.current !== null;
    if (t.current && t.current !== el) giveTitleBack(t.current);
    clearTimers(t);
    t.stopAutoUpdate?.();
    claimOpen(t.self);
    t.current = el;
    t.visible = false;
    t.positioned = false;
    if (!t.panel || !t.shell) ({ shell: t.shell, panel: t.panel } = createPanel(t.doc));
    t.panel.textContent = text;
    t.panel.style.opacity = "0";
    t.doc.body.append(t.shell);
    t.stopAutoUpdate = autoUpdate(el, t.shell, () => void place(t));
    void place(t);
    const delay = handOver || isWarm(t.self) ? 0 : SHOW_DELAY_MS;
    const show = () => {
        t.visible = true;
        paint(t);
    };
    if (delay === 0) show();
    else t.showTimer = setTimeout(show, delay);
}

function scheduleClose(t: TipState): void {
    if (!t.current || t.hideTimer !== null) return;
    t.hideTimer = setTimeout(() => closeNow(t), HIDE_DELAY_MS);
}

function letGo(t: TipState): void {
    if (t.held) giveTitleBack(t.held);
    t.held = null;
}

function onPointerOver(t: TipState, e: Event): void {
    if (t.held) {
        // Still on the element that was clicked: it stays quiet until the pointer leaves it.
        if (e.target instanceof Node && t.held.contains(e.target)) return;
        letGo(t);
    }
    // The title is off the element while the pointer is on it, so inside it the element itself is the candidate.
    const el = candidateFor(e.target) ?? (t.current && e.target instanceof Node && t.current.contains(e.target) ? t.current : null);
    if (!el) return scheduleClose(t);
    if (el === t.current) {
        if (t.hideTimer !== null) {
            clearTimeout(t.hideTimer);
            t.hideTimer = null;
        }
        // A reactive label may have changed while it is open.
        const text = takeTitle(el);
        if (!text) return closeNow(t);
        if (t.panel && t.panel.textContent !== text) t.panel.textContent = text;
        return;
    }
    if (isPrimaryButtonDown()) return;
    const text = takeTitle(el);
    if (!text) return scheduleClose(t);
    openFor(t, el, text);
}

/**
 * Start showing the shared tooltip for titled elements in `doc`. Safe to call twice (the
 * first call wins); returns a function that stops it and removes the panel.
 */
let started: (() => void) | null = null;
export function initTitleTooltips(doc: Document = document): () => void {
    if (started) return started;
    const win = doc.defaultView ?? window;
    const t = createTipState(doc);

    const onOver = (e: Event) => onPointerOver(t, e);
    const onLeave = () => scheduleClose(t);
    // A click, a key or a wheel turn means the pointer is not resting any more.
    const hideNow = () => closeNow(t, true);
    const hideAll = () => {
        closeNow(t);
        letGo(t);
    };

    doc.addEventListener("pointerover", onOver, true);
    doc.addEventListener("pointerdown", hideNow, true);
    doc.addEventListener("keydown", hideNow, true);
    doc.addEventListener("wheel", hideNow, { capture: true, passive: true });
    doc.documentElement.addEventListener("pointerleave", onLeave);
    win.addEventListener("blur", hideAll);

    const stop = () => {
        hideAll();
        doc.removeEventListener("pointerover", onOver, true);
        doc.removeEventListener("pointerdown", hideNow, true);
        doc.removeEventListener("keydown", hideNow, true);
        doc.removeEventListener("wheel", hideNow, true);
        doc.documentElement.removeEventListener("pointerleave", onLeave);
        win.removeEventListener("blur", hideAll);
        started = null;
    };
    started = stop;
    return stop;
}
