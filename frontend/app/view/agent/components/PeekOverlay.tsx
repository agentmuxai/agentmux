// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekOverlay — Portal-rendered hover-to-peek panel, anchored to the TOP
 * edge of the hovered entry (per 2026-08-03 user feedback: "when it appears
 * we need it to appear at the top of the entry") and flush to its left/
 * right edges, but rendered OUTSIDE the virtualized transcript's per-row
 * DOM subtree.
 *
 * Why not a plain `position: absolute` child of the row (what
 * UserMessageBlock.tsx's own overlay used to do, before it was migrated
 * onto this component)? Each row in the virtualized document
 * (`.agent-document-row`) carries `contain: layout` and a `transform`
 * (identity matrix, but any non-`none` transform still counts) — both
 * independently force a NEW STACKING CONTEXT per CSS spec. A `z-index`
 * inside one row's stacking context can never out-rank a LATER SIBLING
 * row's entire subtree, no matter how high the number: sibling stacking
 * contexts stack strictly by DOM order. Confirmed live via CDP:
 * `elementFromPoint` at the overlay's own screen coordinates returned a
 * LATER row's own content, not the overlay itself.
 *
 * Portal-rendering escapes every row's stacking context (renders at
 * `document.body`, in the root stacking context), so this always paints
 * above the whole transcript regardless of virtualization internals — the
 * same reason the original floating-ui `Tooltip` component never had this
 * bug. Position is `fixed` (not `absolute`) using raw
 * `getBoundingClientRect()` viewport coordinates — already zoom-safe (CSS
 * `zoom`, not `transform: scale()`, per AgentDocumentVirtualList.tsx's own
 * CDP-verified comments) — and floating-ui's `autoUpdate` keeps it synced
 * on scroll/resize without hand-rolling scroll listeners.
 *
 * Positioning is deliberately single-direction (top-anchored, growing
 * downward over the entry) rather than the above/below picker
 * hover-anchor.ts's `pickExpandDirection` provides — that picker exists for
 * UserMessageBlock's IN-FLOW body preview, which needs to dodge screen
 * edges since it can be tall (a multi-kB startup payload). This overlay is
 * always a couple of short metadata lines, and the explicit ask is for it
 * to sit at the entry's own top rather than floating below/above it, so
 * there's no direction to pick — height is simply capped to the space
 * between the entry's top and the scroll container's bottom.
 */

import clsx from "clsx";
import { autoUpdate } from "@floating-ui/dom";
import { createSignal, createEffect, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { findScrollContainerRect } from "./hover-anchor";

interface PeekOverlayProps {
    /** Whether the overlay should be mounted right now. */
    show: boolean;
    /**
     * Getter for the hovered row this overlay is anchored to. A getter, not
     * a plain value — the row's own `ref` callback assigns the caller's
     * local `let rowEl` variable AFTER this component's props are first
     * evaluated, so a plain value would freeze at `undefined` forever.
     * `() => rowEl` re-reads the caller's closure on every call.
     */
    rowEl: () => HTMLElement | undefined;
    /**
     * Extra class appended alongside the base `.agent-node-peek-overlay`
     * chrome — for callers whose content needs its own distinct visual
     * identity (e.g. UserMessageBlock.tsx's accent-bordered "Session
     * context" preview) without forking the whole component.
     */
    class?: string;
    children?: JSX.Element;
}

// Reserved space at the scroll container's bottom edge so the overlay's
// own `overflow-y: auto` never sits flush against the pane border — same
// rationale as hover-anchor.ts's `maxOverlayHeight` margin default.
const BOTTOM_MARGIN_PX = 4;

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
        const cap = Math.max(0, container.bottom - rect.top - BOTTOM_MARGIN_PX);
        setFloatingStyle({
            position: "fixed",
            left: `${rect.left}px`,
            top: `${rect.top}px`,
            width: `${rect.width}px`,
            "max-height": `${cap}px`,
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
                <div
                    ref={registerFloating}
                    class={clsx("agent-node-peek-overlay", props.class)}
                    style={floatingStyle()}
                >
                    {props.children}
                </div>
            </Portal>
        </Show>
    );
}

PeekOverlay.displayName = "PeekOverlay";
