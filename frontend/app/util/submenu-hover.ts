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
    /**
     * Whether leaving the trigger or the submenu closes it (after the
     * safe-triangle grace above). Default true, for a dropdown opened by
     * hovering a toolbar button. A submenu inside a menu passes false, as in
     * a native menu: it stays open when the pointer leaves the menu, and
     * closes only when another row at its level is hovered (the caller's
     * peer close), on an outside click, or when its menu closes.
     */
    closeOnLeave?: boolean;
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
    /**
     * Force-open now, bypassing the open delay — the click-to-open
     * counterpart to close(). Without this, a caller that opens by setting
     * its own external state directly (rather than through onTriggerEnter)
     * leaves `isOpen` internally false; the next onTriggerLeave then no-ops
     * (`if (!isOpen) return`) since nothing ever recorded the open, so the
     * safe-triangle close-intent never starts and the submenu is stuck open
     * until an unrelated close path (outside click, Escape, re-click) fires.
     * Calling this instead keeps internal state truthful, so a later mouse
     * leave closes normally like any hover-opened submenu.
     */
    openNow(): void;
    /**
     * A peer row at this level was entered (call with the entering pointer's position). Closes the submenu
     * at once, as close() does, unless the pointer is still inside the safe triangle from where it left this
     * trigger toward the open submenu: then it is on its way in, so the close waits until the pointer leaves
     * the triangle or the safety timeout, and `settled` runs then. Returns true when the close was held (and
     * `settled` will run), false when it closed now (or was not open) and the caller proceeds itself. The
     * pointer reaching the submenu, or a forced close, drops `settled`.
     */
    closeForPeer(pointer: { clientX: number; clientY: number } | undefined, settled?: () => void): boolean;
    /** Tear down all timers/listeners. Call on unmount / menu close. */
    dispose(): void;
}

interface Point {
    x: number;
    y: number;
}

type Pointer = { clientX: number; clientY: number };

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

type SubmenuEl = { getBoundingClientRect(): DOMRect };

/** The submenu's rect, or null while it has no geometry yet (display:none reports a zero rect). */
function usableRect(el: SubmenuEl | null): DOMRect | null {
    const rect = el?.getBoundingClientRect();
    return rect && rect.width > 0 && rect.height > 0 ? rect : null;
}

/**
 * One close attempt. Leaving the safe triangle from `begin`'s origin toward the submenu, or `timeoutMs`
 * passing (a stalled mousemove must never leave the submenu open forever), calls `onExpire` and then the
 * `settle` callback of a peer close that was held for it. `cancel` drops the attempt and that callback.
 */
function createCloseTracker(getRect: () => DOMRect | null, timeoutMs: number, onExpire: () => void) {
    let origin: Point | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let tracking = false;
    let settle: (() => void) | null = null;

    const cancel = () => {
        if (tracking) {
            document.removeEventListener("mousemove", onMove);
            tracking = false;
        }
        if (timer !== null) {
            clearTimeout(timer);
            timer = null;
        }
        origin = null;
        settle = null;
    };
    const expire = () => {
        const settled = settle;
        onExpire();
        settled?.();
    };
    function onMove(e: MouseEvent): void {
        const rect = getRect();
        if (origin && rect && !isInsideSafeTriangle(origin, { x: e.clientX, y: e.clientY }, rect)) expire();
    }
    return {
        cancel,
        /** Start (or restart) the window from where the pointer left. */
        begin(from: Point, onSettle?: () => void) {
            cancel();
            origin = from;
            settle = onSettle ?? null;
            if (getRect()) {
                tracking = true;
                document.addEventListener("mousemove", onMove);
            }
            timer = setTimeout(expire, timeoutMs);
        },
        /** A window is already running: swap its callback. False when none is running. */
        replaceSettle(onSettle?: () => void): boolean {
            if (timer === null) return false;
            settle = onSettle ?? null;
            return true;
        },
    };
}

export function createSubmenuHover(opts: SubmenuHoverOptions): SubmenuHoverController {
    const openDelayMs = opts.openDelayMs ?? DEFAULT_OPEN_DELAY_MS;
    const closeSafetyTimeoutMs = opts.closeSafetyTimeoutMs ?? DEFAULT_CLOSE_SAFETY_TIMEOUT_MS;
    const closeOnLeave = opts.closeOnLeave ?? true;

    let isOpen = false;
    let openTimer: ReturnType<typeof setTimeout> | null = null;
    let submenuEl: SubmenuEl | null = null;
    // Where the pointer last left the trigger while open, kept even when leaving does not close (closeOnLeave
    // false): the apex of the triangle a peer row entered on the way to the submenu is judged against.
    let triggerLeaveOrigin: Point | null = null;
    const tracker = createCloseTracker(() => usableRect(submenuEl), closeSafetyTimeoutMs, () => doClose());

    const clearOpenTimer = () => {
        if (openTimer !== null) {
            clearTimeout(openTimer);
            openTimer = null;
        }
    };

    const doClose = () => {
        tracker.cancel();
        triggerLeaveOrigin = null;
        if (!isOpen) return;
        isOpen = false;
        opts.onClose();
    };

    /** Shared by onTriggerLeave/onSubmenuLeave: start (or restart) the close-intent window. */
    const beginCloseIntent = (e: Pointer) => tracker.begin({ x: e.clientX, y: e.clientY });

    return {
        onTriggerEnter() {
            tracker.cancel();
            triggerLeaveOrigin = null;
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
            triggerLeaveOrigin = { x: e.clientX, y: e.clientY };
            if (closeOnLeave) beginCloseIntent(e);
        },

        onSubmenuEnter() {
            // Arrived — no more approach to protect, stays open until onSubmenuLeave.
            tracker.cancel();
            triggerLeaveOrigin = null;
        },

        onSubmenuLeave(e) {
            if (!isOpen || !closeOnLeave) return;
            beginCloseIntent(e);
        },

        setSubmenuEl(el) {
            submenuEl = el;
        },

        close() {
            clearOpenTimer();
            doClose();
        },

        closeForPeer(pointer, settled) {
            const rect = usableRect(submenuEl);
            const origin = triggerLeaveOrigin;
            const at = pointer && { x: pointer.clientX, y: pointer.clientY };
            if (!isOpen || !origin || !rect || !at || !isInsideSafeTriangle(origin, at, rect)) {
                clearOpenTimer();
                doClose();
                return false;
            }
            if (!tracker.replaceSettle(settled)) tracker.begin(origin, settled);
            return true;
        },

        openNow() {
            clearOpenTimer();
            tracker.cancel();
            triggerLeaveOrigin = null;
            if (isOpen) return;
            isOpen = true;
            opts.onOpen();
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

export interface PeerRegistry {
    /**
     * Register a peer's hover controller. Returns an unregister function —
     * callers own their own cleanup (e.g. Solid's `onCleanup(unregister)`)
     * since this module stays framework-agnostic.
     */
    register(key: string, hover: SubmenuHoverController): () => void;
    /**
     * Close every registered peer except `key`. With the entering pointer's position and `then`, a peer whose
     * submenu the pointer is still heading for stays open a moment (see closeForPeer) and `then` runs once
     * every held peer has closed, or at once when none was held: the entering row opens its own submenu from it.
     */
    closeOthers(key: string, pointer?: { clientX: number; clientY: number }, then?: () => void): void;
}

/**
 * closeOthers for a plain list of controllers (one menu level): closes every peer but `self`, holding the one
 * the pointer is crossing toward, and runs `then` when none is held any more.
 */
export function closePeers(
    peers: Iterable<SubmenuHoverController>,
    self: SubmenuHoverController | null,
    pointer?: { clientX: number; clientY: number },
    then?: () => void
): void {
    let held = 0;
    const settle = () => {
        if (--held === 0) then?.();
    };
    for (const peer of peers) {
        if (peer !== self && peer.closeForPeer(pointer, settle)) held++;
    }
    if (held === 0) then?.();
}

/**
 * Tracks the hover controllers of peer (same-level) menu items/slots so
 * entering one can force-close any other peer's still-open submenu
 * immediately — matching how a native/VS-Code-style menu behaves for an
 * explicit new selection, on top of (not instead of) the safe-triangle
 * grace period each controller gives its OWN trigger-to-submenu approach.
 * Scoped to whatever the caller considers one "level" — one per MenuBody,
 * one per SubMenu, one per pinned-widget-bar row of parent slots. Peers
 * never reach across levels/scopes a caller doesn't register together.
 */
export function createPeerRegistry(): PeerRegistry {
    const peers = new Map<string, SubmenuHoverController>();
    return {
        register(key, hover) {
            peers.set(key, hover);
            return () => peers.delete(key);
        },
        closeOthers(key, pointer, then) {
            closePeers(
                [...peers].filter(([k]) => k !== key).map(([, h]) => h),
                null,
                pointer,
                then
            );
        },
    };
}
