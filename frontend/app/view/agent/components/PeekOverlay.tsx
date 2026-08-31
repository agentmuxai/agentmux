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
    /**
     * How the panel sizes and sits against the anchored row.
     *
     * - `"end"` (default) — **shrink-wraps its content and pins its RIGHT
     *   edge to the row's right edge**, growing leftward. This is what the
     *   metadata peek wants: two short lines (timestamp, token estimate)
     *   shouldn't stretch a full pane's width, and floating them right
     *   keeps them off the text you're actually reading on the left.
     * - `"stretch"` — full row width, left-aligned (the original
     *   behaviour). Only `UserMessageBlock`'s "Session context" body
     *   preview wants this: it renders a real message body, sometimes
     *   kilobytes of it, where full width is the point.
     *
     * Right-alignment is done with `left: rect.right` + a
     * `translateX(-100%)` rather than a `right:` offset, deliberately —
     * `right` would need `window.innerWidth`, which is a different
     * coordinate space from the `getBoundingClientRect()` values
     * everything else here uses, and would break the CSS-`zoom` safety
     * this component's header documents.
     */
    align?: "end" | "stretch";
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
        if ((props.align ?? "end") === "stretch") {
            setFloatingStyle({
                position: "fixed",
                left: `${rect.left}px`,
                top: `${rect.top}px`,
                width: `${rect.width}px`,
                "max-height": `${cap}px`,
            });
            return;
        }
        // Shrink-wrapped and right-anchored. No `width` is set at all, so the
        // stylesheet's `width: max-content` governs; `max-width` still clamps
        // it to the row so a long tool command can't escape the pane.
        setFloatingStyle({
            position: "fixed",
            left: `${rect.right}px`,
            top: `${rect.top}px`,
            transform: "translateX(-100%)",
            "max-width": `${rect.width}px`,
            "max-height": `${cap}px`,
        });
    };

    // reagent P1 on PR #2392: the RAF below used to be un-cancellable and
    // `floatingEl` was never reset, so a rapid hover→leave (very reachable
    // given the 150ms enter-delay that mounts this Portal, followed by a
    // mouseleave within the same animation frame) let a stale RAF fire
    // AFTER the `<Show>` had already unmounted this div — `floatingEl`
    // still pointed at the detached node, `row` was still valid, so
    // `autoUpdate(row, floatingEl, update)` ran anyway and registered
    // scroll/resize listeners nothing would ever clean up, since the
    // owning `onCleanup` (which only fires once per component-level
    // unmount, not per `show` toggle) had already run with
    // `cleanupAutoUpdate` still `null` at that point. Registering
    // `onCleanup` HERE, synchronously inside the ref callback, attaches
    // it to THIS specific `<Show>` branch's own reactive scope — it fires
    // on every unmount of this div (whether from `show` flipping false or
    // the whole component unmounting), 1:1 with `registerFloating`'s own
    // mounts, so there's no toggle-without-matching-cleanup gap left.
    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        const rafId = requestAnimationFrame(() => {
            const row = props.rowEl();
            if (!row || !floatingEl) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(row, floatingEl, update);
        });
        onCleanup(() => {
            cancelAnimationFrame(rafId);
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = null;
            floatingEl = undefined;
        });
    };

    createEffect(() => {
        if (props.show) {
            update();
        }
    });

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
