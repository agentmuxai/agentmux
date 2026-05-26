// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneSizeBadge — small `<width>×<height>` overlay rendered at the
 * bottom-left of a pane while any pane in the tab is being resized.
 *
 * Mounts inside every `<BlockFrame_Default>` (so every visible pane
 * gets one) but only RENDERS while the LayoutModel's `isResizing()`
 * signal is true. The signal is global to the tab — dragging any
 * splitter cascades sizes across siblings, so every pane shows its
 * current rect while the drag is in flight.
 *
 * Spec: docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md.
 */

import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";

import { NodeModel } from "@/layout/index";

interface PaneSizeBadgeProps {
    nodeModel: NodeModel;
    /** Accessor for the pane's outer frame. Measured via ResizeObserver. */
    target: () => HTMLElement | undefined;
}

export const PaneSizeBadge = (props: PaneSizeBadgeProps): JSX.Element => {
    const [size, setSize] = createSignal<{ w: number; h: number } | null>(null);
    let ro: ResizeObserver | null = null;

    onMount(() => {
        const el = props.target();
        if (!el) return;
        // Seed the signal with the current rect so the first paint
        // during a drag shows real numbers — without this, the badge
        // briefly renders empty until the first ResizeObserver tick
        // (which only fires when the size actually changes).
        const r = el.getBoundingClientRect();
        setSize({ w: Math.round(r.width), h: Math.round(r.height) });

        ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                const cr = entry.contentRect;
                setSize({ w: Math.round(cr.width), h: Math.round(cr.height) });
            }
        });
        ro.observe(el);
    });

    onCleanup(() => {
        ro?.disconnect();
        ro = null;
    });

    return (
        // `nodeModel.isResizing()` is wired to `LayoutModel.isResizing`
        // — same memo for every node, so it flips for the whole tab.
        // See `frontend/layout/lib/layoutNodeModels.ts:52`.
        <Show when={props.nodeModel.isResizing() && size()}>
            <div class="pane-size-badge" aria-hidden="true">
                {size()!.w}×{size()!.h}
            </div>
        </Show>
    );
};

PaneSizeBadge.displayName = "PaneSizeBadge";
