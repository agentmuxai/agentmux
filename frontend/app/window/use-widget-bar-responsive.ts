// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useWidgetBarResponsive — responsive 3-tier collapse for the widget bar.
 *
 * SPEC: SPEC_TOPBAR_PROGRESSIVE_COLLAPSE_2026_06_05.md
 *
 * Tier 1 (wide):    labels + "more" text visible
 * Tier 2 (medium):  icon-only — labels hidden, all icons stay on bar
 * Tier 3 (narrow):  overflow — icons + "…more" button for hidden widgets
 *
 * Each tier uses its own hidden measurement mirror so the decision is never
 * based on the already-collapsed visible bar (no oscillation). Extracted
 * from action-widgets.tsx.
 */

import { createSignal, onCleanup, onMount } from "solid-js";

// Minimum tab strip reserved before widget labels are dropped (tier 1→2).
const MIN_TAB_WIDTH = 120;
// Per-tab comfortable width — labels collapse when each tab would fall
// below this. Deliberately above --ws-tab-min (60 px) so labels drop
// first, then tabs continue shrinking.
const TAB_COLLAPSE_RESERVE_PX = 100;
// Tighter reserves used for the tier 2→3 threshold (icon-only bar is
// narrower, so tabs can afford to be a bit squeezed before overflow).
const MIN_TAB_WIDTH_ICON_ONLY = 80;
const TAB_COLLAPSE_RESERVE_ICON_PX = 70;

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
        const buttons = container?.parentElement?.querySelector(
            ".window-action-buttons"
        ) as HTMLElement | null;
        const tabScroll = header.querySelector(".tab-bar-scroll") as HTMLElement | null;
        const measure = () => {
            const labeledW  = mirrorRef?.offsetWidth ?? 0;
            const iconOnlyW = iconMirrorRef?.offsetWidth ?? 0;
            const headerW   = header.clientWidth;
            if (labeledW === 0 || headerW === 0) return;
            const buttonsW = buttons?.offsetWidth ?? 0;
            const tabCount = tabScroll?.querySelectorAll(".tab").length ?? 0;
            const tabsNeeded = Math.max(MIN_TAB_WIDTH, tabCount * TAB_COLLAPSE_RESERVE_PX);
            setTooNarrow(labeledW + buttonsW + tabsNeeded > headerW);
            const tabsNeededIconOnly = Math.max(MIN_TAB_WIDTH_ICON_ONLY, tabCount * TAB_COLLAPSE_RESERVE_ICON_PX);
            const isTooIconOnly = iconOnlyW + buttonsW + tabsNeededIconOnly > headerW;
            if (isTooIconOnly) {
                const pinnedCount = pinnedWidgets().length;
                if (pinnedCount > 0) {
                    // Always-mounted More button probe gives reliable moreBtnW even
                    // before the live More button mounts on first tier-3 entry.
                    const mirrorMoreW = iconMirrorMoreRef?.offsetWidth ?? 0;
                    const moreBtnW = moreButtonRef()?.offsetWidth || mirrorMoreW;
                    // Mirror 2 includes the More button only when unpinned widgets exist;
                    // strip it from iconOnlyW in that case to get pure per-icon width.
                    const iconsOnlyW = moreWidgets().length > 0
                        ? Math.max(0, iconOnlyW - mirrorMoreW)
                        : iconOnlyW;
                    const perIconW = iconsOnlyW / pinnedCount;
                    const availableForIcons = Math.max(0, headerW - buttonsW - tabsNeededIconOnly - moreBtnW);
                    const fitsCount = Math.max(0, Math.floor(availableForIcons / Math.max(1, perIconW)));
                    setClipCount(Math.max(0, pinnedCount - fitsCount));
                } else {
                    setClipCount(0);
                }
            } else {
                setClipCount(0);
            }
        };
        const ro = new ResizeObserver(measure);
        ro.observe(header);
        ro.observe(mirrorRef);
        ro.observe(iconMirrorRef);
        if (iconMirrorMoreRef) ro.observe(iconMirrorMoreRef);
        const mo = tabScroll ? new MutationObserver(measure) : null;
        if (mo && tabScroll) mo.observe(tabScroll, { childList: true });
        measure();
        onCleanup(() => {
            ro.disconnect();
            mo?.disconnect();
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
