// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { atoms } from "@/store/global";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";
import "./notificationbubbles.scss";
import { NotificationItem } from "./notificationitem";
import { useNotification } from "./usenotification";

/** The most toasts shown at once before "+N more". */
export const MAX_VISIBLE_NOTIFICATIONS = 5;

/** The toasts to show, in order: all when `expanded`; otherwise every
 *  persistent one (it stays until closed, so it isn't tucked away) and the
 *  newest of the rest, up to `MAX_VISIBLE_NOTIFICATIONS` in all. */
export function visibleNotifications(all: NotificationType[], expanded: boolean): NotificationType[] {
    const shown = all.filter((n) => !n.hidden);
    if (expanded) return shown;
    const sticky = shown.filter((n) => n.persistent);
    const room = Math.max(0, MAX_VISIBLE_NOTIFICATIONS - sticky.length);
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

    const floatingStyles = {
        position: "fixed",
        right: "58px",
        // Clear of the status bar (~23px) at the window's bottom edge, plus a
        // gap; at 10px the bottom toast sat on top of it.
        bottom: "34px",
        top: "auto",
        left: "auto",
    };

    return (
        <Show when={isOpen() && !notificationPopoverMode()}>
            <div
                style={floatingStyles as any}
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
                            onMouseEnter={() => setHoveredId(notif.id)}
                            onMouseLeave={() => setHoveredId(null)}
                            isBubble={true}
                        />
                    )}
                </For>
            </div>
        </Show>
    );
};

export { NotificationBubbles };
