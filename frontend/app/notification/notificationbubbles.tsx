// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { atoms } from "@/store/global";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import "./notificationbubbles.scss";
import { NotificationItem } from "./notificationitem";
import { placeToasts, TOAST_AVOID_ATTR, type Box, type Placement } from "./toast-placement";
import { ToastAnnouncer } from "./toast-announcer";
import { useNotification } from "./usenotification";

/** The most toasts shown at once before "+N more". */
export const MAX_VISIBLE_NOTIFICATIONS = 5;

/** The toasts to show, in order: all when `expanded`; otherwise at most
 *  `MAX_VISIBLE_NOTIFICATIONS`, persistent ones first (they stay until closed,
 *  so they aren't the ones tucked away), newest first within each kind. */
export function visibleNotifications(all: NotificationType[], expanded: boolean): NotificationType[] {
    const shown = all.filter((n) => !n.hidden);
    if (expanded) return shown;
    const sticky = shown.filter((n) => n.persistent).slice(-MAX_VISIBLE_NOTIFICATIONS);
    const room = MAX_VISIBLE_NOTIFICATIONS - sticky.length;
    const rest = shown.filter((n) => !n.persistent);
    const keep = new Set([...sticky, ...(room > 0 ? rest.slice(-room) : [])]);
    return shown.filter((n) => keep.has(n));
}

const NotificationBubbles = (): JSX.Element => {
    const {
        notifications,
        hoveredId,
        closeNotification,
        copyNotification,
        handleActionClick,
        formatTimestamp,
        setHoveredId,
        pauseToast,
        resumeToast,
    } = useNotification();
    const [isOpen, setIsOpen] = createSignal(notifications().length > 0);
    const [expanded, setExpanded] = createSignal(false);
    const notificationPopoverMode = atoms.notificationPopoverMode;

    createEffect(() => {
        setIsOpen(notifications().length > 0);
    });

    const visible = createMemo(() => visibleNotifications(notifications(), expanded()));
    const more = () => notifications().filter((n) => !n.hidden).length - visible().length;
    // Back to the short list once everything fits again.
    createEffect(() => {
        if (expanded() && notifications().filter((n) => !n.hidden).length <= MAX_VISIBLE_NOTIFICATIONS) {
            setExpanded(false);
        }
    });

    // Clear of the status bar (~23px) at the window's bottom edge, plus a gap; at 10px the bottom
    // toast sat on top of it.
    const USUAL_BOTTOM = 34;
    const RIGHT = 58;
    const TOP = 40;
    const REPLACE_EVERY_MS = 200;
    // Kept off any block a pane or drawer marks with `data-toast-avoid` (toast-placement.ts).
    const [placement, setPlacement] = createSignal<Placement>({ bottom: USUAL_BOTTOM });
    let stack: HTMLDivElement | undefined;
    // A marked block can change size with no DOM change (the composer's textarea grows as you type), so
    // the marked blocks are watched for resizing too; the set is re-synced at every measure.
    let onMarkedResize = () => {};
    const resizeObserver = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => onMarkedResize());
    const watched = new Set<Element>();
    const syncWatched = (marked: Element[]) => {
        for (const el of watched) if (!marked.includes(el)) (resizeObserver?.unobserve(el), watched.delete(el));
        for (const el of marked) if (!watched.has(el)) (resizeObserver?.observe(el), watched.add(el));
    };
    const measure = () => {
        if (!stack) return;
        const marked = [...document.querySelectorAll(`[${TOAST_AVOID_ATTR}]`)];
        syncWatched(marked);
        const avoid: Box[] = marked.map((el) => {
            const r = el.getBoundingClientRect();
            return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        });
        const next = placeToasts({
            viewportWidth: window.innerWidth,
            viewportHeight: window.innerHeight,
            width: stack.offsetWidth,
            height: stack.offsetHeight,
            right: RIGHT,
            bottom: USUAL_BOTTOM,
            top: TOP,
            avoid,
        });
        const prev = placement();
        if (prev.bottom !== next.bottom) setPlacement(next);
    };
    // While toasts show: re-place them when the window, a scroll or the page's layout changes, at most
    // every REPLACE_EVERY_MS (streaming text and animations change the DOM constantly, and a persistent
    // toast can stay up for hours), and not for the stack's own style change.
    createEffect(() => {
        if (!isOpen() || notificationPopoverMode()) return;
        visible();
        let timer: ReturnType<typeof setTimeout> | undefined;
        const schedule = () => {
            if (timer === undefined) timer = setTimeout(() => ((timer = undefined), measure()), REPLACE_EVERY_MS);
        };
        onMarkedResize = schedule;
        const observer = new MutationObserver((records) => {
            if (records.some((m) => !stack || !stack.contains(m.target))) schedule();
        });
        observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style", "hidden", TOAST_AVOID_ATTR] });
        window.addEventListener("resize", schedule);
        window.addEventListener("scroll", schedule, true);
        const first = requestAnimationFrame(measure); // now, so a new toast never shows in the wrong place first
        onCleanup(() => {
            onMarkedResize = () => {};
            resizeObserver?.disconnect();
            watched.clear();
            observer.disconnect();
            window.removeEventListener("resize", schedule);
            window.removeEventListener("scroll", schedule, true);
            clearTimeout(timer);
            cancelAnimationFrame(first);
        });
    });
    const floatingStyles = () => ({
        position: "fixed",
        right: `${RIGHT}px`,
        bottom: `${placement().bottom}px`,
        top: "auto",
        left: "auto",
    });

    return (
        <>
            <ToastAnnouncer notifications={notifications} />
            <Show when={isOpen() && !notificationPopoverMode()}>
                <div
                    ref={stack}
                    style={floatingStyles() as any}
                    class="notification-bubbles"
                    onClick={(e) => e.stopPropagation()}
                >
                    <Show when={more() > 0}>
                        <Button className="notification-more ghost grey" onClick={() => setExpanded(true)}>
                            +{more()} more
                        </Button>
                    </Show>
                    <For each={visible()}>
                        {(notif) => (
                            <NotificationItem
                                class={clsx({ hovered: hoveredId() === notif.id })}
                                notification={notif}
                                onRemove={closeNotification}
                                onCopy={copyNotification}
                                onActionClick={handleActionClick}
                                formatTimestamp={formatTimestamp}
                                onMouseEnter={() => (pauseToast(notif), setHoveredId(notif.id))}
                                onMouseLeave={() => (resumeToast(notif), setHoveredId(null))}
                                isBubble={true}
                            />
                        )}
                    </For>
                </div>
            </Show>
        </>
    );
};

export { NotificationBubbles };
