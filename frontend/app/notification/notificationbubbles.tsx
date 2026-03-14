// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms } from "@/store/global";
import clsx from "clsx";
import { createEffect, createSignal, For, Show, type JSX } from "solid-js";
import "./notificationbubbles.scss";
import { NotificationItem } from "./notificationitem";
import { useNotification } from "./usenotification";

const NotificationBubbles = (): JSX.Element => {
    const {
        notifications,
        hoveredId,
        hideNotification,
        copyNotification,
        handleActionClick,
        formatTimestamp,
        setHoveredId,
    } = useNotification();
    const [isOpen, setIsOpen] = createSignal(notifications().length > 0);
    const notificationPopoverMode = atoms.notificationPopoverMode;

    createEffect(() => {
        setIsOpen(notifications().length > 0);
    });

    const floatingStyles = {
        position: "fixed",
        right: "58px",
        bottom: "10px",
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
                <For each={notifications()}>
                    {(notif) => {
                        if (notif.hidden) return null;
                        return (
                            <NotificationItem
                                class={clsx({ hovered: hoveredId() === notif.id })}
                                notification={notif}
                                onRemove={hideNotification}
                                onCopy={copyNotification}
                                onActionClick={handleActionClick}
                                formatTimestamp={formatTimestamp}
                                onMouseEnter={() => setHoveredId(notif.id)}
                                onMouseLeave={() => setHoveredId(null)}
                                isBubble={true}
                            />
                        );
                    }}
                </For>
            </div>
        </Show>
    );
};

export { NotificationBubbles };
