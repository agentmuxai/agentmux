// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekOverlay — Portal-rendered hover-to-peek panel, styled and positioned
 * like UserMessageBlock.tsx's "Session context" collapsed-row overlay
 * (flush to the row's left/right edges, no gap above/below), but rendered
 * OUTSIDE the virtualized transcript's per-row DOM subtree.
 *
 * Why not a plain `position: absolute` child of the row (what
 * UserMessageBlock.tsx itself does)? Each row in the virtualized document
 * (`.agent-document-row`) carries `contain: layout` and a `transform`
 * (identity matrix, but any non-`none` transform still counts) — both
 * independently force a NEW STACKING CONTEXT per CSS spec. A `z-index`
 * inside one row's stacking context can never out-rank a LATER SIBLING
 * row's entire subtree, no matter how high the number: sibling stacking
 * contexts stack strictly by DOM order. Since a chat transcript has no gap
 * between adjacent rows, any "below" overlay taller than 0 necessarily
 * paints into the space the next row occupies — and that row's own opaque
 * background then paints OVER it, on the very next frame, because it's a
 * later DOM sibling in its own isolated context. Confirmed live via CDP:
 * `elementFromPoint` at the overlay's own screen coordinates returned the
 * NEXT row's `.paragraph` node, not the overlay itself. (This is very
 * likely a latent bug in UserMessageBlock's own overlay too — just rarely
 * triggered there, since a collapsed startup row isn't hovered as often or
 * as densely as tool calls are throughout a long transcript.)
 *
 * Portal-rendering escapes every row's stacking context (renders at
 * `document.body`, in the root stacking context), so this always paints
 * above the whole transcript regardless of virtualization internals — the
 * same reason the original floating-ui `Tooltip` component never had this
 * bug. Position is `fixed` (not `absolute`) using raw
 * `getBoundingClientRect()` viewport coordinates — already zoom-safe (CSS
 * `zoom`, not `transform: scale()`, per AgentDocumentVirtualList.tsx's own
 * CDP-verified comments) — and floating-ui's `autoUpdate` keeps it synced
 * on scroll/resize without hand-rolling scroll listeners. No offset/flip/
 * shift middleware: direction (above/below) and height-capping come from
 * `hover-anchor.ts`'s pure functions, the same ones UserMessageBlock.tsx
 * uses, not floating-ui's own placement logic.
 */

import { autoUpdate } from "@floating-ui/dom";
import { createSignal, createEffect, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import {
    findScrollContainerRect,
    maxOverlayHeight,
    pickExpandDirection,
} from "./hover-anchor";

interface PeekOverlayProps {
    /** Whether the overlay should be mounted right now. */
    show: boolean;
    /**
     * Getter for the hovered row this overlay is anchored to (its left/
     * right edges + top/bottom, per hover-anchor.ts's direction picking).
     * A getter, not a plain value — the row's own `ref` callback assigns
     * the caller's local `let rowEl` variable AFTER this component's props
     * are first evaluated, so a plain value would freeze at `undefined`
     * forever. `() => rowEl` re-reads the caller's closure on every call.
     */
    rowEl: () => HTMLElement | undefined;
    /** Conservative estimated overlay height, for direction selection —
     *  see hover-anchor.ts's `pickExpandDirection` bodyEstimate param. */
    estimateHeightPx: number;
    children?: JSX.Element;
}

export function PeekOverlay(props: PeekOverlayProps): JSX.Element {
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });

    let floatingEl: HTMLElement | undefined;
    let cleanupAutoUpdate: (() => void) | null = null;

    const update = () => {
        const row = props.rowEl();
        if (!row) return;
        const rect = row.getBoundingClientRect();
        const container = findScrollContainerRect(row);
        const rowV = { top: rect.top, bottom: rect.bottom };
        const dir = pickExpandDirection(rowV, container, props.estimateHeightPx);
        const cap = maxOverlayHeight(rowV, container, dir);
        setFloatingStyle({
            position: "fixed",
            left: `${rect.left}px`,
            width: `${rect.width}px`,
            "max-height": `${cap}px`,
            ...(dir === "below"
                ? { top: `${rect.bottom}px` }
                : { bottom: `${window.innerHeight - rect.top}px` }),
        });
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            const row = props.rowEl();
            if (!row || !floatingEl) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(row, floatingEl, update);
        });
    };

    createEffect(() => {
        if (props.show) {
            update();
        } else {
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = null;
        }
    });

    onCleanup(() => cleanupAutoUpdate?.());

    return (
        <Show when={props.show}>
            <Portal>
                <div ref={registerFloating} class="agent-node-peek-overlay" style={floatingStyle()}>
                    {props.children}
                </div>
            </Portal>
        </Show>
    );
}

PeekOverlay.displayName = "PeekOverlay";
