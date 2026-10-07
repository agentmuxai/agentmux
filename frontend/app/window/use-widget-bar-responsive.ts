// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useWidgetBarResponsive — responsive 3-tier collapse for the widget bar.
 *
 * SPEC: SPEC_TOPBAR_PROGRESSIVE_COLLAPSE_2026_06_05.md, with the triggers from
 * SPEC_TOPBAR_LABELS_DROP_BEFORE_TABS_SHRINK_2026_10_07.md (top-bar-tier.ts).
 *
 * Tier 1 (wide):    labels + "more" text visible; tabs at their natural width
 * Tier 2 (medium):  icon-only — labels hidden, all icons stay on bar; tabs shrink
 * Tier 3 (narrow):  tabs at their floor; icons + "…more" button for hidden widgets
 *
 * Each tier uses its own hidden measurement mirror so the decision is never
 * based on the already-collapsed visible bar (no oscillation). Extracted
 * from action-widgets.tsx.
 */

import { createSignal, onCleanup, onMount } from "solid-js";
import { decideTopBarTier } from "./top-bar-tier";

// Fallbacks for the tab bar's CSS variables (tabbar.scss), used only if a
// computed value can't be read: the natural width before a tab is measured
// (TAB_STANDARD_WIDTH), the floor (--ws-tab-min) and the drag gutter.
const TAB_NATURAL_FALLBACK_PX = 232;
const TAB_FLOOR_FALLBACK_PX = 60;
const DRAG_GUTTER_FALLBACK_PX = 32;

const px = (v: string | null | undefined, fallback: number): number => {
    const n = parseFloat(v ?? "");
    return Number.isFinite(n) ? n : fallback;
};

/** An element's layout width plus margins, unrounded, in its own CSS px. */
function outerWidth(el: HTMLElement): number {
    const cs = getComputedStyle(el);
    let w = px(cs.width, el.offsetWidth);
    if (cs.boxSizing !== "border-box") {
        w += px(cs.paddingLeft, 0) + px(cs.paddingRight, 0) + px(cs.borderLeftWidth, 0) + px(cs.borderRightWidth, 0);
    }
    return w + px(cs.marginLeft, 0) + px(cs.marginRight, 0);
}

/**
 * The tab bar's width with every tab at its natural width, and with every tab
 * at its floor: tabs plus separators, the drag gutter after the last tab, and
 * anything else in the tab bar (the hamburger on Windows/Linux). Read from the
 * same inline `--tab-natural-width` and CSS variables the layout uses.
 */
function measureTabStrip(tabBar: HTMLElement, tabScroll: HTMLElement): { naturalPx: number; floorPx: number } {
    const style = getComputedStyle(tabBar);
    const floor = px(style.getPropertyValue("--ws-tab-min"), TAB_FLOOR_FALLBACK_PX);
    const max = px(style.getPropertyValue("--ws-tab-max"), Infinity);
    let natural = 0;
    let tabs = 0;
    for (const w of tabScroll.querySelectorAll<HTMLElement>(":scope > .tab-drop-wrapper")) {
        tabs++;
        natural += Math.min(max, Math.max(floor, px(w.style.getPropertyValue("--tab-natural-width"), TAB_NATURAL_FALLBACK_PX)));
    }
    // Exact widths, not offsetWidth: separators are fractional at a non-100%
    // zoom (v-separator's --snap-chrome), and rounding each one would add up
    // past the tolerance with many tabs.
    let separators = 0;
    for (const sep of tabScroll.querySelectorAll<HTMLElement>(":scope > .tab-separator")) separators += outerWidth(sep);
    const fill = tabScroll.querySelector<HTMLElement>(":scope > .tab-bar-fill");
    const gutter = fill ? px(getComputedStyle(fill).minWidth, DRAG_GUTTER_FALLBACK_PX) : DRAG_GUTTER_FALLBACK_PX;
    // Everything in the tab bar that isn't the scrolling strip (the hamburger).
    const rest = Math.max(0, tabBar.clientWidth - tabScroll.offsetWidth);
    const fixed = rest + separators + gutter;
    return { naturalPx: fixed + natural, floorPx: fixed + tabs * floor };
}

/**
 * Width a flex row spends on everything except `keep`: its padding, the gaps
 * between its laid-out children, every child's margins, and the width of every
 * in-flow child not in `keep`. Hidden and absolutely positioned children take
 * no space.
 */
function spentAround(row: HTMLElement, keep: readonly Element[]): number {
    const style = getComputedStyle(row);
    let spent = px(style.paddingLeft, 0) + px(style.paddingRight, 0);
    let laidOut = 0;
    for (const child of row.children) {
        const el = child as HTMLElement;
        if (el.getClientRects().length === 0) continue;
        const cs = getComputedStyle(el);
        if (cs.position === "absolute" || cs.position === "fixed") continue;
        laidOut++;
        // A kept child's margins still take room; only its own width is shared.
        spent += px(cs.marginLeft, 0) + px(cs.marginRight, 0);
        if (!keep.includes(el)) spent += el.offsetWidth;
    }
    const gap = px(style.columnGap, 0);
    return spent + gap * Math.max(0, laidOut - 1);
}

export function useWidgetBarResponsive(opts: {
    containerRef: () => HTMLDivElement | undefined;
    moreButtonRef: () => HTMLDivElement | undefined;
    pinnedWidgets: () => { key: string; widget: WidgetConfigType }[];
    moreWidgets: () => { key: string; widget: WidgetConfigType }[];
    iconOnly: () => boolean;
}) {
    const { containerRef, moreButtonRef, pinnedWidgets, moreWidgets, iconOnly } = opts;

    const [tooNarrow, setTooNarrow] = createSignal(false); // tier 1→2: drop labels
    const [clipCount, setClipCount] = createSignal(0);     // pinned icons pushed to overflow in tier 3

    // Tier 1 only: show widget labels and the More button's "more" text.
    const showWidgetLabels = () => !tooNarrow() && !iconOnly();

    // Tier 3: split pinned widgets into those that fit on the bar vs. those that overflow.
    const visiblePinnedWidgets = () => {
        const all = pinnedWidgets();
        const clip = clipCount();
        return clip > 0 ? all.slice(0, Math.max(0, all.length - clip)) : all;
    };
    const clippedPinnedWidgets = () => {
        const all = pinnedWidgets();
        const clip = clipCount();
        return clip > 0 ? all.slice(Math.max(0, all.length - clip)) : [];
    };

    let mirrorRef: HTMLDivElement | undefined;
    let iconMirrorRef: HTMLDivElement | undefined;
    let iconMirrorMoreRef: HTMLDivElement | undefined;

    onMount(() => {
        const container = containerRef();
        const header = container?.closest(".window-header") as HTMLElement | null;
        if (!header || !mirrorRef || !iconMirrorRef) return;
        const tabBar = header.querySelector(".tab-bar") as HTMLElement | null;
        const tabScroll = header.querySelector(".tab-bar-scroll") as HTMLElement | null;
        // The status area that holds the widget bar (a direct child of the header).
        const statusArea = container?.parentElement as HTMLElement | null;
        // The widgets give up space first, then the tabs, then the widgets
        // again: labels drop the moment a tab would go below its natural
        // width, and icons overflow only once tabs are at their floor.
        // SPEC_TOPBAR_LABELS_DROP_BEFORE_TABS_SHRINK_2026_10_07.md.
        const measure = () => {
            const labeledW = mirrorRef?.offsetWidth ?? 0;
            const iconOnlyW = iconMirrorRef?.offsetWidth ?? 0;
            if (labeledW === 0 || !container || !tabBar || !tabScroll || !statusArea || header.clientWidth === 0) return;
            const strip = measureTabStrip(tabBar, tabScroll);
            const pinnedCount = pinnedWidgets().length;
            // Always-mounted More button probe gives reliable moreBtnW even
            // before the live More button mounts on first tier-3 entry.
            const mirrorMoreW = iconMirrorMoreRef?.offsetWidth ?? 0;
            const moreBtnW = moreButtonRef()?.offsetWidth || mirrorMoreW;
            // The icon-only mirror includes the More button only when unpinned
            // widgets exist; strip it to get the pure per-icon width.
            const iconsOnlyW = moreWidgets().length > 0 ? Math.max(0, iconOnlyW - mirrorMoreW) : iconOnlyW;
            // The room the tab bar and the widget bar share: the header minus
            // everything else in it and in the status area. Computed from the
            // header, not from the live bars, so it is right even when the
            // live widget bar overflows (a jump straight to a narrow window)
            // and doesn't depend on the tier currently shown.
            const sharedPx =
                header.clientWidth - spentAround(header, [tabBar, statusArea]) - spentAround(statusArea, [container]);
            const tier = decideTopBarTier({
                sharedPx,
                labeledPx: labeledW,
                iconOnlyPx: iconOnlyW,
                tabsNaturalPx: strip.naturalPx,
                tabsFloorPx: strip.floorPx,
                pinnedCount,
                perIconPx: pinnedCount > 0 ? iconsOnlyW / pinnedCount : 1,
                moreButtonPx: moreBtnW,
            });
            setTooNarrow(tier.tooNarrow);
            setClipCount(pinnedCount > 0 ? tier.clipCount : 0);
        };
        // One measurement per frame: a rename or a reorder touches several tabs.
        let raf: number | null = null;
        const schedule = () => {
            if (raf !== null) return;
            raf = requestAnimationFrame(() => {
                raf = null;
                measure();
            });
        };
        const ro = new ResizeObserver(schedule);
        ro.observe(header);
        ro.observe(mirrorRef);
        ro.observe(iconMirrorRef);
        if (iconMirrorMoreRef) ro.observe(iconMirrorMoreRef);
        // Re-check after a tier change lands (idempotent: the decision doesn't
        // depend on the tier shown), and when anything beside the bars resizes.
        if (tabBar) ro.observe(tabBar);
        if (container) ro.observe(container);
        if (statusArea) ro.observe(statusArea);
        // Tabs added or removed, and a tab's natural width changing (its
        // wrapper's inline --tab-natural-width), without the header resizing.
        const mo = tabScroll ? new MutationObserver(schedule) : null;
        if (mo && tabScroll) mo.observe(tabScroll, { childList: true, subtree: true, attributes: true, attributeFilter: ["style"] });
        measure();
        onCleanup(() => {
            ro.disconnect();
            mo?.disconnect();
            if (raf !== null) cancelAnimationFrame(raf);
        });
    });

    return {
        tooNarrow,
        clipCount,
        showWidgetLabels,
        visiblePinnedWidgets,
        clippedPinnedWidgets,
        setMirrorRef: (el: HTMLDivElement) => { mirrorRef = el; },
        setIconMirrorRef: (el: HTMLDivElement) => { iconMirrorRef = el; },
        setIconMirrorMoreRef: (el: HTMLDivElement) => { iconMirrorMoreRef = el; },
    };
}
