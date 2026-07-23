// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Active-tab-color-line measurement, split out of tabbar.tsx: the thin
// colored line rendered under the active tab, which spans the full tab
// strip and re-measures on resize/scroll/tab-order changes.

import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { atoms } from "@/store/global";
import { getWaveObjectAtom, makeORef } from "../store/wos";
import { tabWrapperRefs } from "./tabbar-dnd";

export interface ActiveTabColorLine {
    lineLeft: () => number;
    lineWidth: () => number;
    lineBottom: () => number;
    lineReady: () => boolean;
    activeTabColor: () => string | undefined | null;
}

/**
 * Tracks the active tab's accent color and the pixel geometry of the line
 * rendered under the whole tab strip. Must be called during SolidJS
 * component setup (uses onMount/createEffect/onCleanup internally).
 */
export function useActiveTabColorLine(
    refs: {
        tabBarRef: () => HTMLDivElement;
        tabBarScrollRef: () => HTMLDivElement;
        tabBarFillRef: () => HTMLDivElement;
    },
    tabIds: () => string[],
): ActiveTabColorLine {
    const { tabBarRef, tabBarScrollRef, tabBarFillRef } = refs;
    const activeTabId = atoms.activeTabId;

    // Active tab's color, reactive to both which tab is active AND that
    // tab's own color changing while it stays active — same two-level-memo
    // pattern tabcontent.tsx uses (a plain `getObjectValue` read wouldn't
    // re-subscribe when only the color, not the active id, changes).
    const activeTabAtom = createMemo(() => getWaveObjectAtom<Tab>(makeORef("tab", activeTabId())));
    const activeTabData = createMemo(() => activeTabAtom()());
    const activeTabColor = createMemo((): string | undefined | null => activeTabData()?.meta?.["tab:color"] as string | undefined | null);

    // The line is rendered as a sibling of .tab-bar-scroll (both children of
    // .tab-bar), NOT as a child inside it — .tab-bar-scroll is the horizontal
    // SCROLL container (overflow-x: auto), and an absolutely-positioned
    // descendant of a scroll container moves with its scrollLeft, which
    // would double-count scroll offset against this line's own
    // getBoundingClientRect()-based measurements (a bug reagentx review on
    // #1979 caught). Rendering outside that subtree and re-measuring on
    // scroll (below) instead means the line always reflects wherever the
    // strip's boundaries currently sit on screen — correct whether the
    // strip is scrolled or not, and consistent for both edges.
    //
    // left/right are measured from the first and last tabs' own wrapper
    // elements (tabWrapperRefs, populated by DroppableTab — see
    // tabbar-dnd.ts) and .tab-bar-fill respectively, not any container
    // edge, so the line spans the FULL tab strip — every tab, not just from
    // the selected tab onward — regardless of which tab is selected or how
    // far the strip is scrolled. (Only the line's COLOR follows the
    // selected tab, via activeTabColor() below.) Falls back to leaving the
    // previous values in place if a ref isn't available yet (e.g. a render
    // race right after a tab is created) rather than flashing to some other
    // position.
    // Viewport-absolute px (not relative to any container) — the line can
    // still sit near .tab-bar-scroll's right edge while that container is
    // mid-horizontal-scroll, and .tab-bar has `overflow: hidden`, so it's
    // rendered via a <Portal> to document.body (position: fixed) rather
    // than as a normal .tab-bar child, to escape that clipping.
    // getBoundingClientRect() is already viewport-relative and already
    // post-zoom (`.window-header` applies `zoom` uniformly to everything
    // inside it), so these values are usable directly by a fixed-position
    // element outside that zoomed/clipped subtree with no extra conversion.
    const [lineLeft, setLineLeft] = createSignal(0);
    const [lineWidth, setLineWidth] = createSignal(0);
    const [lineBottom, setLineBottom] = createSignal(0);
    // Gates rendering the line to "we've actually measured the CURRENTLY
    // active tab's real position" — see the retry effect below for why this
    // can briefly be false (new-tab creation) and why showing the line with
    // a stale position in that window is worse than not showing it at all.
    const [lineReady, setLineReady] = createSignal(false);
    const measureLine = (): boolean => {
        const barEl = tabBarRef();
        if (!barEl) return false;
        // Left edge spans the FULL tab strip — the FIRST tab's own left
        // edge, not just from the selected tab rightward. Matches the right
        // edge's "stop exactly at the tabs' own boundary" symmetrically
        // (previously left ran from the active tab while right ran to the
        // strip's end, an asymmetric span). The active tab only decides the
        // line's COLOR (activeTabColor(), a data read below — independent
        // of any DOM ref), not its extent, so activeTabEl's rect is no
        // longer needed here.
        const firstTabEl = tabWrapperRefs.get(tabIds()[0]);
        if (!firstTabEl) return false;
        const left = firstTabEl.getBoundingClientRect().left;
        setLineLeft(left);
        setLineBottom(window.innerHeight - barEl.getBoundingClientRect().bottom);

        // Right edge stops where the actual tabs stop, not the viewport
        // edge. `.tab-bar-fill` is the flex-filler <div> rendered as the
        // last child of .tab-bar-scroll, immediately after the last tab
        // with no separator in between — its own left edge is therefore,
        // by construction, flush with the last tab's right edge regardless
        // of tab count, tab widths (content-aware sizing — SPEC_TAB_CONTENT_AWARE_SIZING_2026-06-14.md),
        // or scroll position. Falls back to the viewport edge only if the
        // ref genuinely isn't available (shouldn't happen — .tab-bar-fill
        // is always rendered — but matches the fallback style already used
        // elsewhere in this file rather than silently producing NaN).
        // Reverses an earlier deliberate choice (commit d1e990d9) to run
        // the line all the way to the viewport edge, under the header
        // widgets and window controls — see
        // specs/SPEC_ACTIVE_TAB_COLOR_LINE_STOP_AT_TAB_STRIP_2026_07_13.md
        // for why that's being narrowed to the tab strip's own boundary.
        const right = tabBarFillRef()?.getBoundingClientRect().left ?? window.innerWidth;
        setLineWidth(right - left);
        return true;
    };
    // Re-measure whenever the selected tab (its color drives the line, even
    // though its position no longer does) or the tab order (a reorder drag,
    // or a tab added/removed at either end, shifts the strip's own
    // boundaries) changes.
    //
    // A newly-added tab's DroppableTab hasn't necessarily mounted (and
    // registered itself in tabWrapperRefs — see tabbar-dnd.ts) by the time
    // this effect's dependencies update — most notably when it lands at
    // index 0 and `tabIds()[0]` now points at a ref that doesn't exist yet.
    // Without retrying, `measureLine` bailed and left the PREVIOUS
    // left/width in place, so the line rendered at a stale position. Retry
    // across a few animation frames until the ref shows up, hiding the line
    // meanwhile (lineReady) rather than showing it at that stale position.
    //
    // Precise tracking mid-drag-reorder (the 100ms gap-padding transition
    // in tabbar.scss) is intentionally out of scope here — this settles
    // correctly once the drag/transition ends.
    createEffect(() => {
        // Reads establish this effect's reactive deps — re-runs (and, via
        // the onCleanup below, cancels any still-in-flight retry loop from
        // a superseded selection) whenever either changes.
        activeTabId();
        tabIds();
        let cancelled = false;
        let attempts = 0;
        const tryMeasure = () => {
            if (cancelled) return;
            if (measureLine()) {
                setLineReady(true);
                return;
            }
            if (attempts >= 10) return; // give up quietly after ~10 frames
            attempts++;
            requestAnimationFrame(tryMeasure);
        };
        setLineReady(false);
        tryMeasure();
        onCleanup(() => {
            cancelled = true;
        });
    });
    onMount(() => {
        if (measureLine()) setLineReady(true);
        const ro = new ResizeObserver(() => measureLine());
        ro.observe(tabBarRef());
        const scrollEl = tabBarScrollRef();
        if (scrollEl) {
            ro.observe(scrollEl);
            scrollEl.addEventListener("scroll", measureLine);
        }
        const fillEl = tabBarFillRef();
        if (fillEl) ro.observe(fillEl);
        window.addEventListener("resize", measureLine);
        onCleanup(() => {
            ro.disconnect();
            scrollEl?.removeEventListener("scroll", measureLine);
            window.removeEventListener("resize", measureLine);
        });
    });

    return { lineLeft, lineWidth, lineBottom, lineReady, activeTabColor };
}
