// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { For, Show, type JSX } from "solid-js";

import "./notificationitem.scss";

interface NotificationItemProps {
    notification: NotificationType;
    onRemove: (id: string) => void;
    onCopy: (id: string) => void;
    onActionClick: (e: MouseEvent, action: NotificationActionType, id: string) => void;
    formatTimestamp: (timestamp: string) => string;
    isBubble: boolean;
    class?: string;
    onMouseEnter?: () => void;
    onMouseLeave?: () => void;
}

const NotificationItem = ({
    notification,
    onRemove,
    onCopy,
    onActionClick,
    formatTimestamp,
    isBubble,
    class: className,
    onMouseEnter,
    onMouseLeave,
}: NotificationItemProps): JSX.Element => {
    const { id, title, message, icon, type, timestamp, persistent, actions } = notification;
    const color = type === "error" ? "red" : type === "warning" ? "yellow" : "green";
    const nIcon = icon ? icon : "bell";

    const renderCloseButton = () => {
        if (!isBubble && persistent) {
            return (
                <span class="lock-btn" title="Cannot be cleared">
                    <i class={makeIconClass("lock", false)}></i>
                </span>
            );
        }
        return (
            <Button
                className="close-btn ghost grey py-[10px]"
                onClick={(e) => {
                    e.stopPropagation();
                    onRemove(id);
                }}
                aria-label="Close"
            >
                <i class={clsx(makeIconClass("close", false), color)}></i>
            </Button>
        );
    };

    return (
        <div
            class={clsx(isBubble ? "notification-bubble" : "notification", className)}
            onMouseEnter={onMouseEnter}
            onMouseLeave={onMouseLeave}
            onClick={() => onCopy(id)}
            title="Click to Copy Notification Message"
        >
            {renderCloseButton()}
            <div class="notification-inner">
                <Show when={nIcon}>
                    <div class="notification-icon">
                        <i class={clsx(makeIconClass(nIcon, false), color)}></i>
                    </div>
                </Show>
                <div class="notification-text">
                    <Show when={title}>
                        <div class={clsx("notification-title", color)}>{title}</div>
                    </Show>
                    <Show when={timestamp && !isBubble}>
                        <div class="notification-timestamp">{formatTimestamp(timestamp)}</div>
                    </Show>
                    <Show when={message}>
                        <div class="notification-message">{message}</div>
                    </Show>
                    <Show when={actions && actions.length > 0}>
                        <div class="notification-actions">
                            <For each={actions}>
                                {(action, index) => (
                                    <Button
                                        onClick={(e) => onActionClick(e, action, id)}
                                        className={clsx(
                                            action.color,
                                            "py-[4px] px-[8px] text-[13px] rounded-[4px]"
                                        )}
                                        disabled={action.disabled}
                                    >
                                        {action.label}
                                        <Show when={action.rightIcon}>
                                            <i class={makeIconClass(action.rightIcon, false)}></i>
                                        </Show>
                                    </Button>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    );
};

export { NotificationItem };
