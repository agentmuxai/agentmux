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
 * Positioning ("end" mode) is peek-placement.ts's job, and the panel always sits
 * near the pointer. Horizontally: right-aligned to the row when it fits the row's
 * width; pinned to the row's left edge and extending right over the pane border,
 * up to the window edge, when wider. Vertically: below or above the pointer
 * inside the transcript when it fits; otherwise it leaves the transcript, on the
 * side of the pointer with more room, with its HEIGHT cut to that room so it can
 * never reach the pointer, and the rest scrolls. The old code clamped the
 * position into the container and let the panel grow back over the pointer,
 * which flickers (the row sees `mouseleave`, the panel closes, the row sees
 * `mouseenter`, it reopens).
 *
 * A panel that had to be cut scrolls, so it is meant to be ENTERED, and four
 * things make sure the pointer can get there:
 *  1. it is PINNED once it turns out to scroll: it stops following the pointer
 *     (a panel that follows moves away from a pointer heading for it, and its
 *     height, cut to the room left, shrinks as it does);
 *  2. its near edge is FLUSH with the row (`flushToRow`), so the way to it does
 *     not cross a strip of the next row;
 *  3. it lingers after the row's `mouseleave`, and the linger is re-armed while
 *     the pointer keeps closing in on it (HOVER_BRIDGE_MS, up to
 *     HOVER_BRIDGE_MAX_MS), then stays while the pointer is on it;
 *  4. while the pointer is crossing to it, other rows' peeks wait
 *     (peek-bridge.ts), so one it passes over on the way cannot open on top.
 * A panel that fits behaves as before: it follows the pointer and closes the
 * instant the pointer leaves the row.
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
import { BOTTOM_MARGIN_PX, computePeekHorizontal, computePeekVertical } from "./peek-placement";
import {
    beginPeekBridge,
    clearPeekPanelOpenAfterLeave,
    endPeekBridge,
    markPeekPanelOpenAfterLeave,
} from "./peek-bridge";

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
 * How long an enterable panel lingers after the pointer leaves its row, so the
 * pointer can reach it. Re-armed on every move that brings the pointer closer
 * to the panel, so a slow or diagonal approach still makes it; a pointer moving
 * away lets it close after this long. Entering the panel then holds it open
 * (scroll bar, text selection). Standard hover-card grace.
 */
const HOVER_BRIDGE_MS = 150;
/** The longest an approaching pointer can keep the grace period going, in total. */
const HOVER_BRIDGE_MAX_MS = 1000;

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

    // True when the panel reaches past the pane (wider than the row, or taller
    // than the transcript). Only then does it carry `data-pane-overlay`, so the
    // browser-pane airspace cut (platform/pane-overlay-auto.ts) is not driven for
    // every short peek.
    const [outside, setOutside] = createSignal(false);
    // True when the last `update()` had to cut the panel's height, so it scrolls.
    // Only such a panel is meant to be ENTERED (scroll bar, text selection).
    let lastEnterable = false;

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
    let bridgeStartedAt = 0;
    let closestApproach = Infinity;
    // The row this panel bridged, remembered rather than re-read at release time,
    // so a row element that changes meanwhile cannot leave the bridge set (which
    // would hold off every other peek).
    let bridgedRow: HTMLElement | undefined;
    const releaseBridge = () => {
        endPeekBridge(bridgedRow);
        bridgedRow = undefined;
    };

    // How far the pointer is from the panel (0 inside it).
    const distanceToPanel = (x: number, y: number): number => {
        const r = floatingEl?.getBoundingClientRect();
        if (!r) return Infinity;
        const dx = Math.max(r.left - x, 0, x - r.right);
        const dy = Math.max(r.top - y, 0, y - r.bottom);
        return Math.hypot(dx, dy);
    };
    // During the grace period: a move that brings the pointer closer than it has
    // been re-arms the timer (up to HOVER_BRIDGE_MAX_MS in total).
    const onApproach = (e: MouseEvent) => {
        const d = distanceToPanel(e.clientX, e.clientY);
        if (d < closestApproach) {
            closestApproach = d;
            armBridgeTimer();
        }
    };
    const clearBridgeTimer = () => {
        if (bridgeTimer !== undefined) clearTimeout(bridgeTimer);
        bridgeTimer = undefined;
        document.removeEventListener("mousemove", onApproach);
    };
    const armBridgeTimer = () => {
        if (bridgeTimer !== undefined) clearTimeout(bridgeTimer);
        const budget = HOVER_BRIDGE_MAX_MS - (Date.now() - bridgeStartedAt);
        bridgeTimer = setTimeout(endLinger, Math.max(0, Math.min(HOVER_BRIDGE_MS, budget)));
    };
    const endLinger = () => {
        clearBridgeTimer();
        if (!held && !props.show) {
            releaseBridge();
            setOpen(false);
        }
    };
    const startLinger = () => {
        clearBridgeTimer();
        bridgeStartedAt = Date.now();
        closestApproach = Infinity;
        // Other rows' peeks wait while the pointer crosses to this panel.
        bridgedRow = props.rowEl();
        beginPeekBridge(bridgedRow);
        document.addEventListener("mousemove", onApproach, { passive: true });
        armBridgeTimer();
    };
    createComputed(
        on(
            () => props.show,
            (show) => {
                clearBridgeTimer();
                if (show) {
                    // The pointer is back on the row: nothing is being crossed.
                    releaseBridge();
                    setOpen(true);
                } else if (bridges() && untrack(open) && lastEnterable) {
                    // Only a panel that had to be cut (it scrolls) is meant to be
                    // ENTERED, so only it lingers. A panel that fits has nothing
                    // to scroll and closes the instant the pointer leaves, exactly
                    // as it always did.
                    startLinger(); // stay open; the timer closes it
                } else {
                    setOpen(false);
                }
            },
            { defer: true },
        ),
    );
    onCleanup(clearBridgeTimer);

    // The panel is on screen although its row is no longer hovered (lingering, or
    // the pointer on it). The caller's `isPeeking()` is false then, so tell it,
    // through peek-bridge.ts, that the content must stay: callers gate content on
    // `useNodePeek().panelVisible()`. The row is remembered so the same one is
    // cleared even if the caller's row element changes meanwhile.
    let markedRow: HTMLElement | undefined;
    createComputed(() => {
        const openAfterLeave = open() && !props.show;
        untrack(() => {
            if (openAfterLeave) {
                markedRow = props.rowEl();
                markPeekPanelOpenAfterLeave(markedRow);
            } else if (markedRow) {
                clearPeekPanelOpenAfterLeave(markedRow);
                markedRow = undefined;
            }
        });
    });
    onCleanup(() => {
        clearPeekPanelOpenAfterLeave(markedRow);
        markedRow = undefined;
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
    // Pointer Y relative to the row's top, frozen the first time the panel turns
    // out to be enterable (it scrolls). From then until it closes, placement uses
    // this instead of the live pointer, so the panel holds still while the pointer
    // moves toward it, and it still moves with the row if the transcript scrolls.
    // Following the live pointer made a tall panel unreachable: each move toward
    // it moved it away and cut its height again.
    let pinnedOffsetY: number | null = null;

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
        // Shrink-wrapped, near the pointer. Horizontal and vertical are decided
        // by peek-placement.ts (see its header): right-aligned to the row, or
        // pinned left and extending over the pane border when wider than the
        // row; below/above the pointer, leaving the transcript and cutting its
        // height to the room when too tall. The invariant it keeps — the
        // pointer is never inside the panel — is swept by its tests.
        const viewport = { width: window.innerWidth, height: window.innerHeight };
        const rowRect = { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
        const hz = computePeekHorizontal({ row: rowRect, viewport, naturalWidth: measureWidth() });
        const placeAt = { row: rowRect, container, viewport, naturalHeight: measureHeightAt(hz.maxWidth, paneZoom) };
        let vt: ReturnType<typeof computePeekVertical>;
        if (pinnedOffsetY != null) {
            vt = computePeekVertical({ ...placeAt, mouseY: rect.top + pinnedOffsetY, flushToRow: true });
        } else {
            vt = computePeekVertical({ ...placeAt, mouseY: lastMouseY });
            // Enterable: pin it, and keep its near edge on the row so the way to
            // it does not cross another row. Decided on the unpinned placement
            // and then sticky until it closes, so the two never alternate.
            if (vt.scrolls && lastMouseY != null) {
                pinnedOffsetY = lastMouseY - rect.top;
                vt = computePeekVertical({ ...placeAt, mouseY: lastMouseY, flushToRow: true });
            }
        }
        // Enterable if pinned, or if it scrolls with no pointer Y to pin to yet
        // (a peek opened by the drag-release hover resync before any mousemove:
        // it still needs its linger, and pins on the first move).
        lastEnterable = pinnedOffsetY != null || vt.scrolls;
        setOutside(hz.extendsPastRow || vt.leavesContainer);
        setFloatingStyle(
            withPaneZoom(
                {
                    position: "fixed",
                    left: `${hz.left}px`,
                    top: `${vt.top}px`,
                    ...(hz.alignRight ? { transform: "translateX(-100%)" } : {}),
                    "max-width": `${hz.maxWidth}px`,
                    "max-height": `${vt.maxHeight}px`,
                },
                paneZoom,
            ),
        );
    };

    // The panel measured with its caps lifted. Inline styles are set and
    // restored synchronously, so nothing paints between. With the height cap
    // lifted `getBoundingClientRect` IS the natural size, in real viewport px
    // at any pane zoom (unlike `scrollHeight`, which is in unzoomed px).
    //
    // Width first, with no width cap: the content's max-content width, which
    // does not depend on the layout it is currently in. Then the height at the
    // width the horizontal placement chose. Measuring at fixed inputs means the
    // placement never depends on the placement chosen last time.
    const withCapsLifted = <T,>(maxWidth: string, read: (el: HTMLElement) => T, fallback: T): T => {
        const el = floatingEl;
        if (!el) return fallback;
        const prevW = el.style.maxWidth;
        const prevH = el.style.maxHeight;
        el.style.maxWidth = maxWidth;
        el.style.maxHeight = "none";
        const out = read(el);
        el.style.maxWidth = prevW;
        el.style.maxHeight = prevH;
        return out;
    };
    const measureWidth = (): number =>
        withCapsLifted("none", (el) => el.getBoundingClientRect().width, 0);
    const measureHeightAt = (maxWidthPx: number, paneZoom: number): number =>
        withCapsLifted(`${maxWidthPx / paneZoom}px`, (el) => el.getBoundingClientRect().height, 0);

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
                // A pinned panel does not follow the pointer (see pinnedOffsetY).
                if (props.show && pinnedOffsetY == null) update();
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
            // Closed: the next opening places itself afresh, and nothing is
            // being crossed to any more.
            pinnedOffsetY = null;
            lastEnterable = false;
            clearBridgeTimer();
            releaseBridge();
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
                        // Arrived: the crossing is over. (Rows under the panel
                        // cannot be hovered while the pointer is on it anyway, so
                        // no bridge is needed here, and none can go stale if a
                        // mouseleave is ever missed.)
                        releaseBridge();
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
