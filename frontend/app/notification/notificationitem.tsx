// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { fadeOutCopy } from "@/app/element/exit-copy";
import { useMotion } from "@/app/motion/solid";
import { Button } from "@/element/button";
import { IconButton } from "@/app/element/ui";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import "./notificationitem.scss";

/** How long "Copied" stays on the Copy button. */
const COPIED_MS = 1500;

interface NotificationItemProps {
    notification: NotificationType;
    onRemove: (id: string) => void;
    /** Copies the toast (title, full message and debug details); resolves when it is on the clipboard. */
    onCopy: (id: string) => void | Promise<void>;
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
    // Hovering and keyboard focus each pause the toast's expiry; it resumes only when neither holds.
    let hovering = false;
    let focused = false;
    const syncPause = () => (hovering || focused ? onMouseEnter?.() : onMouseLeave?.());
    let root: HTMLDivElement | undefined;
    // "Copied" shows for a moment after a copy; focus on the button also holds the toast open.
    const [copied, setCopied] = createSignal(false);
    let copiedTimer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(copiedTimer));
    const copy = () => {
        void Promise.resolve(onCopy(id))
            .then(() => {
                setCopied(true);
                clearTimeout(copiedTimer);
                copiedTimer = setTimeout(() => setCopied(false), COPIED_MS);
            })
            .catch(() => {});
    };
    // A toast slides in when it appears and a copy of it fades out where it was when it goes
    // (the stack removes it at once, so the others close up without waiting).
    if (isBubble) {
        const motion = useMotion();
        onMount(() => root && motion.enter(root, "slideY"));
        onCleanup(() => root && fadeOutCopy(root, "slideY", { fixed: true }));
    }

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
            ref={root}
            class={clsx(isBubble ? "notification-bubble" : "notification", className)}
            onMouseEnter={() => {
                hovering = true;
                syncPause();
            }}
            onMouseLeave={() => {
                hovering = false;
                syncPause();
            }}
            onFocusIn={() => {
                focused = true;
                syncPause();
            }}
            onFocusOut={(e) => {
                // Focus moving between this toast's own buttons is still focus inside it.
                if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
                focused = false;
                syncPause();
            }}
            onClick={copy}
        >
            {renderCloseButton()}
            <span class={clsx("copy-btn", copied() && "copied")}>
                <IconButton
                    icon={copied() ? "check" : "copy"}
                    label="Copy details"
                    tooltipPlacement="left"
                    onClick={(e: MouseEvent) => {
                        e.stopPropagation();
                        copy();
                    }}
                />
                {/* Outside the button, whose name would otherwise hide it from a screen reader. */}
                <span class="copy-btn-status" role="status">
                    {copied() ? "Copied" : ""}
                </span>
            </span>
            <Show when={isBubble && persistent}>
                <span class="pin-btn" title="Stays until you close it" aria-label="Stays until you close it">
                    <i class={makeIconClass("thumbtack", false)}></i>
                </span>
            </Show>
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
