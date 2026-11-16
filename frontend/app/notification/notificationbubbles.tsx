// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { t } from "@/app/i18n/i18n";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { focusManager } from "@/app/store/focusManager";
import { Button } from "@/element/ui";
import { atoms } from "@/store/global";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";
import "./notificationbubbles.scss";
import { NotificationItem } from "./notificationitem";
import { ToastAnnouncer } from "./toast-announcer";
import { toastRight, useToastPlacement } from "./use-toast-placement";
import { closeAllNotifications, useNotification } from "./usenotification";

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

/** The stack's box. It cuts a hole through a browser pane's native window it floats over, so the toasts show above
 *  the page. `data-pane-overlay` does that on Windows alone (pane-overlay-auto.ts); `usePaneOverlay` does it on
 *  every platform, and the same rect registered twice on Windows is one hole. It mounts only while there is a
 *  stack, so the hole is there for exactly as long. */
const ToastStackBox = (props: { setStack: (el: HTMLDivElement) => void; style: JSX.CSSProperties; children: JSX.Element }): JSX.Element => {
    let box: HTMLDivElement | undefined;
    usePaneOverlay(() => box);
    return (
        <div
            ref={(el) => {
                box = el;
                props.setStack(el);
            }}
            style={props.style}
            class="notification-bubbles"
            data-pane-overlay
            onClick={(e) => e.stopPropagation()}
        >
            {props.children}
        </div>
    );
};

/** The stack's own row, above the toasts and, in a stack that scrolls, held at its top edge (sticky): how many
 *  toasts are not shown, when any, and "Dismiss all". */
const ToastStackBar = (props: { more: number; onShowMore: () => void; onDismissAll: () => void }): JSX.Element => (
    <div class="notification-stack-bar">
        <Show when={props.more > 0}>
            <Button class="notification-more" tone="quiet" aria-expanded="false" onClick={props.onShowMore}>
                {t("toasts.more", { count: props.more })}
            </Button>
        </Show>
        <Button
            class="notification-dismiss-all"
            density="compact"
            // A pointer press leaves the caret where it is (a composer, say).
            onMouseDown={(e) => e.preventDefault()}
            onClick={props.onDismissAll}
        >
            {t("toasts.dismissAll")}
        </Button>
    </div>
);

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
    const shownCount = () => notifications().filter((n) => !n.hidden).length;
    const more = () => shownCount() - visible().length;
    // Back to the short list once everything fits again.
    createEffect(() => {
        if (expanded() && shownCount() <= MAX_VISIBLE_NOTIFICATIONS) {
            setExpanded(false);
        }
    });

    const { bottom, maxHeight, setStack: setPlacedStack } = useToastPlacement(() => isOpen() && !notificationPopoverMode(), visible);
    let stack: HTMLDivElement | undefined;
    const setStack = (el: HTMLDivElement) => {
        stack = el;
        setPlacedStack(el);
    };
    /** "Dismiss all": every toast goes, the ones behind "+N more" included. The button goes with them, so a caret
     *  that was in the stack (the button was reached by keyboard) returns to the selected pane, not to <body>. */
    const dismissAll = () => {
        const caretInStack = !!stack?.contains(document.activeElement);
        closeAllNotifications();
        if (caretInStack) focusManager.ensureSelectionFocused("toasts-dismissed");
    };
    // The stack is an unzoomed shell placed in real pixels; the body inside it carries the chrome zoom.
    const floatingStyles = () => ({
        position: "fixed",
        right: `${toastRight()}px`,
        bottom: `${bottom()}px`,
        top: "auto",
        left: "auto",
    });

    return (
        <>
            <ToastAnnouncer notifications={notifications} />
            <Show when={isOpen() && !notificationPopoverMode()}>
                <ToastStackBox setStack={setStack} style={floatingStyles() as JSX.CSSProperties}>
                    <div
                        class="notification-bubbles-body"
                        style={maxHeight() === undefined ? undefined : { "max-height": `calc(${maxHeight()}px / var(--zoomfactor))` }}
                    >
                        <Show when={shownCount() > 1}>
                            <ToastStackBar more={more()} onShowMore={() => setExpanded(true)} onDismissAll={dismissAll} />
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
                </ToastStackBox>
            </Show>
        </>
    );
};

export { NotificationBubbles };
