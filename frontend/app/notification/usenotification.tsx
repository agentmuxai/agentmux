// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, setNotifications } from "@/store/global";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createRoot, createSignal, untrack } from "solid-js";
import { useTick } from "@/app/hook/useTick";

const notificationActions: { [key: string]: () => void } = {
    installUpdate: () => {
        getApi().installAppUpdate();
    },
    // Add other action functions here
};

// Actions registered at runtime live only as long as a notification offers them: once a
// notification that offered one leaves the list (expired, dismissed, or its button used), the
// action goes too. Built-in actions above are never pruned.
const runtimeActions = new Set<string>();
const offeredRuntimeActions = new Set<string>();
let pruning = false;

function startPruningRuntimeActions(): void {
    if (pruning) return;
    pruning = true;
    createRoot(() =>
        createEffect(() => {
            const offered = new Set(atoms.notifications().flatMap((n) => (n.actions ?? []).map((a) => a.actionKey)));
            for (const key of [...runtimeActions]) {
                if (offered.has(key)) {
                    offeredRuntimeActions.add(key);
                } else if (offeredRuntimeActions.has(key)) {
                    // Offered before and gone now. A key registered but not yet offered is kept.
                    clearNotificationAction(key);
                }
            }
        })
    );
}

/** Register (or replace) the function a notification action button runs, for actions
 *  whose effect is only known when the notification is raised (an Undo, say). It is
 *  forgotten once no notification offers it any more. */
export function setNotificationAction(actionKey: string, fn: () => void): void {
    notificationActions[actionKey] = fn;
    runtimeActions.add(actionKey);
    startPruningRuntimeActions();
}

/** Forget an action registered with `setNotificationAction`. */
export function clearNotificationAction(actionKey: string): void {
    if (!runtimeActions.has(actionKey)) return;
    runtimeActions.delete(actionKey);
    offeredRuntimeActions.delete(actionKey);
    delete notificationActions[actionKey];
}

/** True while an action key has a handler; exported for tests. */
export function hasNotificationAction(actionKey: string): boolean {
    return actionKey in notificationActions;
}

export function useNotification() {
    const notificationPopoverMode = atoms.notificationPopoverMode;
    const notifications = atoms.notifications;
    const [hoveredId, setHoveredId] = createSignal<string | null>(null);

    const removeNotification = (id: string) => {
        setNotifications((prevNotifications) => prevNotifications.filter((n) => n.id !== id));
    };

    /** The X: the toast goes, and its sender hears it was closed. */
    const closeNotification = (id: string) => {
        const notif = untrack(notifications).find((n) => n.id === id);
        removeNotification(id);
        notif?.onClose?.();
    };

    const hideNotification = (id: string) => {
        setNotifications((prevNotifications) =>
            prevNotifications.map((n) => (n.id === id ? { ...n, hidden: true } : n))
        );
    };

    const hideAllNotifications = () => {
        setNotifications((prevNotifications) => prevNotifications.map((n) => ({ ...n, hidden: true })));
    };

    const removeAllNotifications = () => {
        setNotifications((prevNotifications) => prevNotifications.filter((n) => n.persistent));
    };

    const copyNotification = (id: string) => {
        const notif = notifications().find((n) => n.id === id);
        if (!notif) return;

        let text = notif.title ?? "";
        if (notif.message) {
            text += text.length > 0 ? `\n${notif.message}` : notif.message;
        }
        clipboardWriteText(text)
            .then(() => {
                console.info("Text copied to clipboard");
            })
            .catch((err) => {
                console.error("Failed to copy text: ", err);
            });
    };

    const handleActionClick = (e: MouseEvent, action: NotificationActionType, id: string) => {
        e.stopPropagation();
        const actionFn = notificationActions[action.actionKey];
        if (actionFn) {
            actionFn();
            removeNotification(id);
        } else {
            console.warn(`No action found for key: ${action.actionKey}`);
        }
    };

    const tick = useTick(1000);
    // All reads inside are untracked — tick() is the only reactive source.
    // Without untrack, reading notifications() here and then writing via
    // setNotifications() creates a self-dependency that loops unboundedly.
    createEffect(() => {
        tick();
        if (untrack(notificationPopoverMode)) return;
        const notifs = untrack(notifications);
        if (!notifs.some((n) => n.expiration)) return;
        const now = Date.now();
        setNotifications((prev) =>
            prev.filter((n) => !n.expiration || n.persistent || n.expiration > now || n.id === untrack(hoveredId))
        );
    });

    const formatTimestamp = (timestamp: string): string => {
        const notificationTime = new Date(timestamp).getTime();
        const now = Date.now();
        const diffInSeconds = Math.floor((now - notificationTime) / 1000);
        const diffInMinutes = Math.floor(diffInSeconds / 60);
        const diffInHours = Math.floor(diffInMinutes / 60);
        const diffInDays = Math.floor(diffInHours / 24);

        if (diffInMinutes === 0) {
            return `Just now`;
        } else if (diffInMinutes < 60) {
            return `${diffInMinutes} mins ago`;
        } else if (diffInHours < 24) {
            return `${diffInHours} hrs ago`;
        } else if (diffInDays < 7) {
            return `${diffInDays} days ago`;
        } else {
            return new Date(timestamp).toLocaleString();
        }
    };

    return {
        notifications,
        hoveredId,
        setHoveredId,
        removeNotification,
        closeNotification,
        removeAllNotifications,
        hideNotification,
        hideAllNotifications,
        copyNotification,
        handleActionClick,
        formatTimestamp,
    };
}
