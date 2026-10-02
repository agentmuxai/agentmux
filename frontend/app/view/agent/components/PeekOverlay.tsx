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
 * `getBoundingClientRect()` viewport coordinates, and floating-ui's
 * `autoUpdate` keeps it synced on scroll/resize without hand-rolling
 * scroll listeners.
 *
 * **Pane zoom (2026-09-06).** Escaping the row's subtree also escapes the
 * pane's `zoom`, which `agent-view.tsx` sets on the pane root — so this
 * panel used to render at 100% no matter how far the agent pane was zoomed
 * in or out, while everything around it scaled. An earlier revision of this
 * header called the component "already zoom-safe", which was true only of
 * its POSITION: `getBoundingClientRect()` reports post-zoom viewport pixels,
 * so the panel always landed in the right place — at the wrong size. That
 * half-truth is precisely why the gap survived. It now reads the pane's
 * factor off the anchor row's inherited `--agent-pane-zoom` and applies it
 * to itself; see `readPaneZoom` and `withPaneZoom` for the mechanism and
 * for why every length has to be divided by the factor.
 *
 * Positioning ("end" mode) is peek-placement.ts's job: inside the pane, below or
 * above the pointer, when the panel fits on one side of it (the common short
 * peek, unchanged); beside the pane when it is too tall for either side, so it
 * cannot cover the pointer and may extend past the pane to the window edge; or
 * above/below with its height constrained when there is no room beside. The
 * old code clamped the position into the container and let the panel grow back
 * over the pointer, which flickers (the row sees `mouseleave`, the panel
 * closes, the row sees `mouseenter`, it reopens). A panel placed beside the pane
 * is meant to be entered (scroll bar, text selection), so it lingers briefly
 * after the row's `mouseleave` and stays while the pointer is on it. Tall
 * panels scroll inside a height capped at a share of the window.
 * docs/reports/REPORT_TOOL_HOVER_PANEL_SIZE_AND_PLACEMENT_2026_10_02.md
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
import { createSignal, createComputed, createEffect, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { findScrollContainerRect } from "./hover-anchor";
import { BOTTOM_MARGIN_PX, computePeekPlacement, type PeekMode } from "./peek-placement";

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
     * everything else here uses. It also keeps the pane-zoom compensation
     * in `withPaneZoom` uniform: every length it touches is a
     * `getBoundingClientRect()`-derived viewport pixel, divided by the same
     * factor. A `right:` offset would need the opposite correction.
     */
    align?: "end" | "stretch";
    children?: JSX.Element;
}

/**
 * How long the panel lingers after the pointer leaves the row, so the pointer
 * can cross the gap to a `beside` panel and enter it. Entering the panel then
 * holds it open (scroll bar, text selection). Standard hover-card grace.
 */
const HOVER_BRIDGE_MS = 150;

/**
 * This overlay's owning pane's zoom factor, read off the anchor row.
 *
 * `agent-view.tsx` sets BOTH `zoom: <factor>` and `--agent-pane-zoom:
 * <factor>` on the pane root. The row is inside that subtree, so the custom
 * property inherits down to it — reading from the row means this works for
 * whichever pane the hovered entry belongs to (and for the history view,
 * which sets the same pair) without threading a prop through all six
 * call sites.
 *
 * Returns 1 for anything unparseable, absent, or non-positive: a peek
 * rendered outside an agent pane (or in a test harness with no computed
 * custom properties) must keep behaving exactly as it did before.
 */
function readPaneZoom(row: HTMLElement | undefined): number {
    if (!row) return 1;
    const raw = getComputedStyle(row).getPropertyValue("--agent-pane-zoom").trim();
    const parsed = Number.parseFloat(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

/** Style lengths that are in real viewport px and must be de-scaled. */
const ZOOM_SCALED_LENGTHS = ["left", "top", "width", "max-width", "max-height"] as const;

/**
 * Re-express a style computed in REAL viewport pixels so it renders
 * identically on an element that carries `zoom: paneZoom`.
 *
 * This is the whole fix, and the division is the non-obvious half of it.
 * CSS `zoom` on an element multiplies the used value of that element's OWN
 * lengths — including the inset properties of a positioned element. With
 * `zoom: 2`, `left: 100px` paints at 200px from the viewport's left edge.
 * Every input here comes from `getBoundingClientRect()`, which already
 * reports post-zoom rendered pixels, so each length must be pre-divided by
 * the factor to land where it did before.
 *
 * `transform: translateX(-100%)` is deliberately NOT touched: it resolves
 * against the element's own border-box width, which scales with it, so it
 * stays correct at any zoom.
 *
 * A `paneZoom` of exactly 1 returns the style untouched (no `zoom`
 * property emitted at all) — the overwhelmingly common case stays
 * byte-identical to the pre-fix behavior.
 */
function withPaneZoom(style: JSX.CSSProperties, paneZoom: number): JSX.CSSProperties {
    if (paneZoom === 1) return style;
    const scaled: JSX.CSSProperties = { ...style, zoom: paneZoom };
    for (const key of ZOOM_SCALED_LENGTHS) {
        const value = style[key];
        if (typeof value !== "string") continue;
        const px = Number.parseFloat(value);
        if (!Number.isFinite(px)) continue;
        scaled[key] = `${px / paneZoom}px`;
    }
    return scaled;
}

export function PeekOverlay(props: PeekOverlayProps): JSX.Element {
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });

    // True when the panel has left the pane's own area (`beside`/`vertical`).
    // Only then does it carry `data-pane-overlay`, so the browser-pane airspace
    // cut (platform/pane-overlay-auto.ts) is not driven for every short peek.
    const [outside, setOutside] = createSignal(false);
    // Which placement mode the last `update()` chose. Only `inside`/`vertical`
    // depend on the pointer's Y; `beside` does not, so it skips mouse updates.
    let lastMode: PeekMode = "inside";

    // Hover bridge ("end" mode only). The panel is portalled, so moving onto it
    // fires the row's `mouseleave` and the caller turns `show` off. Keep it up
    // for HOVER_BRIDGE_MS after that, and while the pointer is on it, so the
    // pointer can cross the gap and use the scroll bar / select text. The
    // caller's `show` stays authoritative: when it goes true again (pointer back
    // on the row) the linger is cancelled.
    //
    // `open` is the ONLY thing `<Show>` reads, written synchronously by a
    // computation. Deriving "show || lingering" from two signals instead made the
    // panel unmount for one update (show false, linger not yet set) and remount
    // when the linger arrived — a flicker of its own.
    const bridges = () => (props.align ?? "end") !== "stretch";
    const [open, setOpen] = createSignal(props.show);
    let held = false;
    let bridgeTimer: ReturnType<typeof setTimeout> | undefined;
    const clearBridgeTimer = () => {
        if (bridgeTimer !== undefined) clearTimeout(bridgeTimer);
        bridgeTimer = undefined;
    };
    const startLinger = () => {
        clearBridgeTimer();
        bridgeTimer = setTimeout(() => {
            bridgeTimer = undefined;
            if (!held && !props.show) setOpen(false);
        }, HOVER_BRIDGE_MS);
    };
    createComputed(
        on(
            () => props.show,
            (show) => {
                clearBridgeTimer();
                if (show) {
                    setOpen(true);
                } else if (bridges() && untrack(open) && lastMode !== "inside") {
                    // Only a panel placed to be ENTERED (beside / vertical) lingers.
                    // An `inside` panel sits a cursor-gap away and a short peek has
                    // nothing to scroll, so it closes the instant the pointer
                    // leaves, exactly as it always did.
                    startLinger(); // stay open; the timer closes it
                } else {
                    setOpen(false);
                }
            },
            { defer: true },
        ),
    );
    onCleanup(clearBridgeTimer);

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
        const paneZoom = readPaneZoom(row);
        if ((props.align ?? "end") === "stretch") {
            const cap = Math.max(0, container.bottom - rect.top - BOTTOM_MARGIN_PX);
            setFloatingStyle(
                withPaneZoom(
                    {
                        position: "fixed",
                        left: `${rect.left}px`,
                        top: `${rect.top}px`,
                        width: `${rect.width}px`,
                        "max-height": `${cap}px`,
                    },
                    paneZoom,
                ),
            );
            return;
        }
        // Shrink-wrapped. Where it goes (inside the pane, beside it, or above/
        // below with a constrained height) is peek-placement.ts's job; the
        // invariant it keeps — the pointer is never inside the panel — is
        // swept by its tests. See that file for why each mode exists.
        const measured = measureNaturalHeight(rect.width, paneZoom);
        const placement = computePeekPlacement({
            row: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
            mouseY: lastMouseY,
            container,
            viewport: { width: window.innerWidth, height: window.innerHeight },
            naturalHeight: measured.natural,
            currentHeight: measured.current,
        });
        lastMode = placement.mode;
        setOutside(placement.mode !== "inside");
        setFloatingStyle(
            withPaneZoom(
                {
                    position: "fixed",
                    left: `${placement.left}px`,
                    top: `${placement.top}px`,
                    ...(placement.alignRight ? { transform: "translateX(-100%)" } : {}),
                    "max-width": `${placement.maxWidth}px`,
                    "max-height": `${placement.maxHeight}px`,
                },
                paneZoom,
            ),
        );
    };

    // The panel's height with no height cap, laid out at the ROW's width — one
    // fixed width, so the mode it picks never depends on the mode it picked
    // last time (a wider `beside` panel is shorter, which would otherwise
    // flip it back to `inside`). `current` is the height as rendered now.
    // Styles are set and restored synchronously, so nothing paints between.
    const measureNaturalHeight = (rowWidth: number, paneZoom: number): { natural: number; current: number } => {
        const el = floatingEl;
        if (!el) return { natural: 0, current: 0 };
        const current = el.getBoundingClientRect().height;
        const prevW = el.style.maxWidth;
        const prevH = el.style.maxHeight;
        el.style.maxWidth = `${rowWidth / paneZoom}px`;
        el.style.maxHeight = "none";
        const chrome = el.offsetHeight - el.clientHeight;
        const natural = Math.max(el.getBoundingClientRect().height, el.scrollHeight + chrome);
        el.style.maxWidth = prevW;
        el.style.maxHeight = prevH;
        return { natural: Math.max(natural, current), current };
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
                if (props.show && lastMode !== "beside") update();
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
        <Show when={open()}>
            <Portal>
                <div
                    ref={registerFloating}
                    class={clsx("agent-node-peek-overlay", props.class)}
                    data-pane-overlay={outside() ? "" : undefined}
                    onMouseEnter={() => {
                        if (!bridges()) return;
                        clearBridgeTimer();
                        held = true;
                    }}
                    onMouseLeave={() => {
                        if (!bridges()) return;
                        held = false;
                        // Pointer left the panel: linger briefly (it may be heading
                        // back to the row, whose mouseenter will re-show it).
                        if (!props.show) startLinger();
                    }}
                    style={floatingStyle()}
                >
                    {props.children}
                </div>
            </Portal>
        </Show>
    );
}

PeekOverlay.displayName = "PeekOverlay";
