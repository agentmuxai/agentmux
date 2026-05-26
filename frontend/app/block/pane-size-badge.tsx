// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneSizeBadge — small `<width>×<height>` overlay rendered at the
 * bottom-left of a pane while any pane in the tab is being resized.
 *
 * Lifetime is gated by the caller (see `blockframe.tsx` — wraps the
 * mount in `<Show when={nodeModel.isResizing()}>`), so this component
 * only exists during a drag. The ResizeObserver attaches on mount
 * (drag start) and detaches on unmount (drag end); zero work runs in
 * the idle steady state. Codex P2 on PR #1057.
 *
 * Both the seed measurement and the observer callback read the **border
 * box** so the displayed value never jumps when the first ResizeObserver
 * tick lands — `getBoundingClientRect()` and `entry.borderBoxSize[0]`
 * agree, while `entry.contentRect` would report ~2px smaller because
 * `.block-frame-default` has `padding: 1px`. Codex P2 on PR #1057.
 *
 * Spec: docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md.
 */

import { createSignal, onCleanup, onMount, type JSX } from "solid-js";

interface PaneSizeBadgeProps {
    /** Accessor for the pane's outer frame. Measured via ResizeObserver. */
    target: () => HTMLElement | undefined;
}

export const PaneSizeBadge = (props: PaneSizeBadgeProps): JSX.Element => {
    const [size, setSize] = createSignal<{ w: number; h: number }>({ w: 0, h: 0 });
    let ro: ResizeObserver | null = null;

    onMount(() => {
        const el = props.target();
        if (!el) return;
        // Seed with the border-box rect synchronously so the very first
        // paint of the badge shows real numbers (without the seed the
        // ResizeObserver fires once after mount, briefly showing 0×0).
        const r = el.getBoundingClientRect();
        setSize({ w: Math.round(r.width), h: Math.round(r.height) });

        ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                // borderBoxSize matches getBoundingClientRect (border-box).
                // contentRect excludes padding and would show a 2px jump
                // on the first tick against the seed value.
                const box = entry.borderBoxSize?.[0];
                if (box) {
                    setSize({ w: Math.round(box.inlineSize), h: Math.round(box.blockSize) });
                } else {
                    // Safari < 17 / older Chromium fallback — borderBoxSize
                    // missing from the entry payload. Re-read the element
                    // directly to keep the same box model.
                    const rr = el.getBoundingClientRect();
                    setSize({ w: Math.round(rr.width), h: Math.round(rr.height) });
                }
            }
        });
        ro.observe(el);
    });

    onCleanup(() => {
        ro?.disconnect();
        ro = null;
    });

    return (
        <div class="pane-size-badge" aria-hidden="true">
            {size().w}×{size().h}
        </div>
    );
};

PaneSizeBadge.displayName = "PaneSizeBadge";
