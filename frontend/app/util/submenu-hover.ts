// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared hover-intent core for submenu open/close timing —
// SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10.md §4.
//
// Framework-agnostic (no Solid/React dependency) so both FlyoutMenu's
// SolidJS SubMenu and showJsContextMenu's vanilla-DOM submenu route
// through the same open-delay + safe-triangle close logic instead of each
// hand-rolling instant, zero-grace-period open/close.

const DEFAULT_OPEN_DELAY_MS = 90;
const DEFAULT_CLOSE_SAFETY_TIMEOUT_MS = 300;

export interface SubmenuHoverOptions {
    /** Delay before the submenu becomes visible after a sustained hover. Default 90. */
    openDelayMs?: number;
    /**
     * How long the cursor may sit outside the safe-triangle polygon (or, with
     * no submenu geometry yet, how long any leave is tolerated) before the
     * submenu closes. Default 300.
     */
    closeSafetyTimeoutMs?: number;
    /** Caller shows the submenu (still visibility-hidden until positioned). */
    onOpen: () => void;
    /** Caller hides/unmounts the submenu. */
    onClose: () => void;
}

export interface SubmenuHoverController {
    /** Call on the trigger row's mouseenter/pointerenter. */
    onTriggerEnter(): void;
    /** Call on the trigger row's mouseleave/pointerleave. */
    onTriggerLeave(e: { clientX: number; clientY: number }): void;
    /**
     * Call on the submenu panel's own mouseenter/pointerenter — the cursor
     * has arrived, so any pending close (timer or safe-triangle tracking) is
     * cancelled and the submenu stays open until the panel itself is left.
     */
    onSubmenuEnter(): void;
    /** Call on the submenu panel's own mouseleave/pointerleave. */
    onSubmenuLeave(e: { clientX: number; clientY: number }): void;
    /**
     * Register (or clear, with `null`) the submenu's DOM element. Its rect is
     * read fresh on every tracked mousemove, so callers don't need to re-call
     * this after `autoUpdate` repositions the same element mid-hover.
     */
    setSubmenuEl(el: { getBoundingClientRect(): DOMRect } | null): void;
    /**
     * Force-close now (cancelling any pending open too), bypassing the open
     * delay and safe-triangle grace period entirely. For an explicit new
     * selection elsewhere — e.g. a peer/sibling item being entered — there is
     * no "approach" to protect, so this closes unconditionally rather than
     * waiting on timers or polygon tracking. The controller stays alive and
     * reusable for a future onTriggerEnter (unlike dispose()).
     */
    close(): void;
    /** Tear down all timers/listeners. Call on unmount / menu close. */
    dispose(): void;
}

interface Point {
    x: number;
    y: number;
}

function sign(p1: Point, p2: Point, p3: Point): number {
    return (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y);
}

/** Standard barycentric-sign point-in-triangle test. */
function pointInTriangle(pt: Point, v1: Point, v2: Point, v3: Point): boolean {
    const d1 = sign(pt, v1, v2);
    const d2 = sign(pt, v2, v3);
    const d3 = sign(pt, v3, v1);
    const hasNeg = d1 < 0 || d2 < 0 || d3 < 0;
    const hasPos = d1 > 0 || d2 > 0 || d3 > 0;
    return !(hasNeg && hasPos);
}

/**
 * The safe triangle: apex at the point the cursor left the trigger row, base
 * along whichever submenu edge faces that point. Submenus in this app always
 * open to the left or right of their trigger (never above/below), so the
 * near edge is picked by horizontal side; the vertical fallback covers any
 * future top/bottom placement without needing a second code path.
 */
function isInsideSafeTriangle(apex: Point, cursor: Point, submenuRect: DOMRect): boolean {
    let corner1: Point;
    let corner2: Point;
    if (apex.x <= submenuRect.left) {
        corner1 = { x: submenuRect.left, y: submenuRect.top };
        corner2 = { x: submenuRect.left, y: submenuRect.bottom };
    } else if (apex.x >= submenuRect.right) {
        corner1 = { x: submenuRect.right, y: submenuRect.top };
        corner2 = { x: submenuRect.right, y: submenuRect.bottom };
    } else if (apex.y <= submenuRect.top) {
        corner1 = { x: submenuRect.left, y: submenuRect.top };
        corner2 = { x: submenuRect.right, y: submenuRect.top };
    } else {
        corner1 = { x: submenuRect.left, y: submenuRect.bottom };
        corner2 = { x: submenuRect.right, y: submenuRect.bottom };
    }
    return pointInTriangle(cursor, apex, corner1, corner2);
}

export function createSubmenuHover(opts: SubmenuHoverOptions): SubmenuHoverController {
    const openDelayMs = opts.openDelayMs ?? DEFAULT_OPEN_DELAY_MS;
    const closeSafetyTimeoutMs = opts.closeSafetyTimeoutMs ?? DEFAULT_CLOSE_SAFETY_TIMEOUT_MS;

    let isOpen = false;
    let openTimer: ReturnType<typeof setTimeout> | null = null;
    let closeTimer: ReturnType<typeof setTimeout> | null = null;
    let submenuEl: { getBoundingClientRect(): DOMRect } | null = null;
    let leaveOrigin: Point | null = null;
    let tracking = false;

    const clearOpenTimer = () => {
        if (openTimer !== null) {
            clearTimeout(openTimer);
            openTimer = null;
        }
    };

    /** Cancel any in-flight close attempt (timer and/or polygon tracking). Does not affect isOpen. */
    const cancelPendingClose = () => {
        if (tracking) {
            document.removeEventListener("mousemove", handleTrackedMouseMove);
            tracking = false;
        }
        if (closeTimer !== null) {
            clearTimeout(closeTimer);
            closeTimer = null;
        }
        leaveOrigin = null;
    };

    const doClose = () => {
        cancelPendingClose();
        if (!isOpen) return;
        isOpen = false;
        opts.onClose();
    };

    function handleTrackedMouseMove(e: MouseEvent): void {
        if (!leaveOrigin) return;
        const rect = submenuEl?.getBoundingClientRect();
        if (!rect || rect.width <= 0 || rect.height <= 0) return;
        const cursor = { x: e.clientX, y: e.clientY };
        if (!isInsideSafeTriangle(leaveOrigin, cursor, rect)) {
            doClose();
        }
    }

    /** Shared by onTriggerLeave/onSubmenuLeave: start (or restart) the close-intent window. */
    const beginCloseIntent = (e: { clientX: number; clientY: number }) => {
        cancelPendingClose();
        leaveOrigin = { x: e.clientX, y: e.clientY };
        const rect = submenuEl?.getBoundingClientRect();
        if (rect && rect.width > 0 && rect.height > 0) {
            tracking = true;
            document.addEventListener("mousemove", handleTrackedMouseMove);
        }
        // Absolute backstop either way — a stalled/undetected mousemove must
        // never leave the submenu open forever.
        closeTimer = setTimeout(doClose, closeSafetyTimeoutMs);
    };

    return {
        onTriggerEnter() {
            cancelPendingClose();
            if (isOpen || openTimer !== null) return;
            openTimer = setTimeout(() => {
                openTimer = null;
                isOpen = true;
                opts.onOpen();
            }, openDelayMs);
        },

        onTriggerLeave(e) {
            clearOpenTimer();
            if (!isOpen) return;
            beginCloseIntent(e);
        },

        onSubmenuEnter() {
            // Arrived — no more approach to protect, stays open until onSubmenuLeave.
            cancelPendingClose();
        },

        onSubmenuLeave(e) {
            if (!isOpen) return;
            beginCloseIntent(e);
        },

        setSubmenuEl(el) {
            submenuEl = el;
        },

        close() {
            clearOpenTimer();
            doClose();
        },

        // Same as close() — if this was open, onClose() MUST fire so the
        // caller's own state (e.g. a visibleSubMenus map entry) stays in
        // sync with reality. Skipping it left a stale visible:true entry
        // for an unmounted-while-open submenu, which then rendered
        // instantly (skipping the open delay) the next time its ancestor
        // reopened (reagent P2 on PR #2525).
        dispose() {
            clearOpenTimer();
            doClose();
        },
    };
}
