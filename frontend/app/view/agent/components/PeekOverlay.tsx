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
 *
 * `align="end"` mode additionally tracks the mouse's Y position while
 * hovering (2026-09-03, SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md):
 * horizontal position stays pinned to the row's right edge exactly as
 * before, but `top` follows the cursor (offset by CURSOR_GAP_PX below it —
 * see `update()`'s own comment for why the offset must be nonzero) instead
 * of freezing at the row's top edge, clamped to the scroll container's own
 * bounds. `align="stretch"` (UserMessageBlock's full-body preview) is
 * unaffected — still top-anchored.
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

// Vertical clearance kept between the cursor and the mouse-tracking overlay's
// own top edge (align="end" mode only) — see the `top` computation in
// `update()` below for why this must be strictly positive.
const CURSOR_GAP_PX = 12;

export function PeekOverlay(props: PeekOverlayProps): JSX.Element {
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });

    let floatingEl: HTMLElement | undefined;
    let cleanupAutoUpdate: (() => void) | null = null;
    // Latest mouse Y within the hovered row, tracked continuously (not
    // gated on `show`) so the panel already has a real position for its
    // very first render instead of flashing at rect.top first. Plain
    // closure var, not a signal — applied via direct style writes, same as
    // the rest of this component's positioning.
    let lastMouseY: number | null = null;
    let mouseMoveRaf: number | null = null;

    const update = () => {
        const row = props.rowEl();
        if (!row) return;
        const rect = row.getBoundingClientRect();
        const container = findScrollContainerRect(row);
        if ((props.align ?? "end") === "stretch") {
            const cap = Math.max(0, container.bottom - rect.top - BOTTOM_MARGIN_PX);
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
        //
        // `top` tracks the mouse's Y (clamped to the scroll container's own
        // bounds) instead of freezing at rect.top — SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md.
        // Horizontal pinning (left/transform) is untouched. Falls back to
        // rect.top if no mouse position is known yet.
        //
        // `top` is offset CURSOR_GAP_PX below the raw cursor position, not
        // exactly at it. First cut of this fix set `top: mouseY` exactly,
        // which put the cursor precisely on the panel's own top edge — any
        // further downward movement immediately entered the (Portal-rendered,
        // non-descendant-of-the-row) overlay itself, firing the row's
        // onMouseLeave and hiding it, which then re-triggered onMouseEnter at
        // the new Y and looped (reagent P1 on PR #2949, 2nd round). The fix
        // tried next — `pointer-events: none` on the overlay — traded that
        // loop for a real regression: `.agent-node-peek-overlay` has a load-
        // bearing `overflow-y: auto` (ToolBlock.tsx's `cmdText` body can be
        // long enough to need scrolling), which pointer-events: none also
        // disables (reagent P1, 3rd round).
        //
        // Placing the panel BELOW-with-a-gap isn't enough on its own,
        // though: clamping `top` to fit the overlay's height within the
        // container (so `max-height` doesn't collapse near the bottom edge)
        // can push `top` back down to <= the cursor's raw Y whenever the
        // cursor is within `overlayHeight + BOTTOM_MARGIN_PX` of the
        // container's bottom — silently reintroducing the exact
        // cursor-inside-the-overlay loop CURSOR_GAP_PX exists to prevent
        // (reagent P1, 4th round: hovering the lowest transcript row, or any
        // tall ToolBlock peek near the pane's bottom, hit this). Below-with-
        // gap and the container-fit clamp can genuinely conflict — there is
        // no single `top` that satisfies both that close to the edge — so
        // this flips to ABOVE-with-a-gap instead of clamping when the
        // below-placement wouldn't fit, the standard tooltip flip-direction
        // pattern. Both branches keep the cursor strictly outside
        // `[top, top + overlayHeight]` by construction (by `CURSOR_GAP_PX`),
        // rather than relying on a clamp that can silently violate that
        // invariant.
        const overlayHeight = floatingEl?.getBoundingClientRect().height ?? 0;
        const minTop = container.top;
        const containerBottomLimit = container.bottom - BOTTOM_MARGIN_PX;
        let top: number;
        if (lastMouseY != null) {
            const belowTop = lastMouseY + CURSOR_GAP_PX;
            if (belowTop + overlayHeight <= containerBottomLimit) {
                top = Math.max(belowTop, minTop);
            } else {
                // Not enough room below the cursor to fit the overlay without
                // clipping — flip above it instead. Still clamped to minTop
                // for the degenerate case where the container itself is
                // shorter than the overlay; some clipping is unavoidable
                // there (BOTTOM_MARGIN_PX/`cap` below still bound it), but
                // that's an existing edge case, not one this fix introduces.
                top = Math.max(lastMouseY - CURSOR_GAP_PX - overlayHeight, minTop);
            }
        } else {
            top = rect.top;
        }
        const cap = Math.max(0, container.bottom - top - BOTTOM_MARGIN_PX);
        setFloatingStyle({
            position: "fixed",
            left: `${rect.right}px`,
            top: `${top}px`,
            transform: "translateX(-100%)",
            "max-width": `${rect.width}px`,
            "max-height": `${cap}px`,
        });
    };

    // Track the mouse continuously while the row exists, independent of
    // `show` (the 50ms enter-delay in useNodePeek means `show` flips true
    // slightly after hover starts — this way the first visible frame
    // already has a real Y instead of one captured from a stale/absent
    // mousemove). rAF-coalesced so fast mouse movement doesn't write
    // styles once per raw event — same pattern `registerFloating` below
    // already uses for its own rAF-gated setup.
    createEffect(() => {
        const row = props.rowEl();
        if (!row || (props.align ?? "end") === "stretch") return;
        const onMouseMove = (e: MouseEvent) => {
            lastMouseY = e.clientY;
            if (mouseMoveRaf != null) return;
            mouseMoveRaf = requestAnimationFrame(() => {
                mouseMoveRaf = null;
                if (props.show) update();
            });
        };
        row.addEventListener("mousemove", onMouseMove);
        onCleanup(() => {
            row.removeEventListener("mousemove", onMouseMove);
            if (mouseMoveRaf != null) {
                cancelAnimationFrame(mouseMoveRaf);
                mouseMoveRaf = null;
            }
        });
    });

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
