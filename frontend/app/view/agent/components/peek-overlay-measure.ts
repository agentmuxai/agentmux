// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Measuring the panel at fixed inputs, so the placement never depends on the placement chosen last time. */

import { READABLE_WIDTH_CHARS } from "./peek-placement";
import type { PeekState } from "./peek-overlay-state";

export function createPeekMeasure(s: PeekState) {
    // The panel measured with its caps lifted. Inline styles are set and
    // restored synchronously, so nothing paints between. With the height cap
    // lifted `getBoundingClientRect` IS the natural size, in real viewport px
    // at any pane zoom (unlike `scrollHeight`, which is in unzoomed px).
    //
    // Width first, with no width cap: the content's max-content width, which
    // does not depend on the layout it is currently in. Then the height at the
    // width the horizontal placement chose. Measuring at fixed inputs means the
    // placement never depends on the placement chosen last time.
    const withCapsLifted = <T,>(maxWidth: string, minWidth: string, read: (el: HTMLElement) => T, fallback: T): T => {
        const el = s.floatingEl;
        if (!el) return fallback;
        const prevW = el.style.maxWidth;
        const prevMin = el.style.minWidth;
        const prevH = el.style.maxHeight;
        el.style.maxWidth = maxWidth;
        el.style.minWidth = minWidth;
        el.style.maxHeight = "none";
        const out = read(el);
        el.style.maxWidth = prevW;
        el.style.minWidth = prevMin;
        el.style.maxHeight = prevH;
        return out;
    };
    // The content's own width: no cap and no minimum.
    const measureWidth = (): number =>
        withCapsLifted("none", "0px", (el) => el.getBoundingClientRect().width, 0);
    // `READABLE_WIDTH_CHARS` of the panel's code font, in viewport px (so at the
    // pane's zoom and font size); 0 when there is no panel to measure in.
    const measureReadableWidth = (): number => {
        const el = s.floatingEl;
        if (!el) return 0;
        const probe = document.createElement("span");
        probe.style.cssText = `position:absolute;visibility:hidden;font-family:var(--font-mono);width:${READABLE_WIDTH_CHARS}ch`;
        el.appendChild(probe);
        const w = probe.getBoundingClientRect().width;
        probe.remove();
        return w;
    };
    // The height at the width the panel will actually have (between min and max).
    const measureHeightAt = (maxWidthPx: number, minWidthPx: number, paneZoom: number): number =>
        withCapsLifted(`${maxWidthPx / paneZoom}px`, `${minWidthPx / paneZoom}px`, (el) => el.getBoundingClientRect().height, 0);

    return { measureWidth, measureReadableWidth, measureHeightAt };
}

export type PeekMeasure = ReturnType<typeof createPeekMeasure>;
