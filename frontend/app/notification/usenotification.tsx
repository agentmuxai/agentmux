// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, setNotifications } from "@/store/global";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createRoot, createSignal, untrack } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { toastDebugText, type ToastDebugEnv } from "./toast-debug";
import { windowId } from "@/app/store/window-identity";
import { formatLocale } from "@/app/i18n/i18n";
import { formatRelativeTime } from "@/util/format-time";

/** A notification older than this shows its date, not "N days ago". */
const NOTIFICATION_AGO_DAYS = 7;

/** The real version, channel, window and tab, read when the button is pressed. */
export function liveToastDebugEnv(): ToastDebugEnv {
    const about = getApi().getAboutModalDetails?.();
    let tabId: string | undefined;
    try {
        tabId = atoms.activeTabId() || undefined;
    } catch {
        tabId = undefined;
    }
    return {
        version: about?.version,
        build: about?.buildLabel,
        channel: about?.channel,
        platform: [about?.platform, about?.arch].filter(Boolean).join(" ") || undefined,
        windowId: windowId() || undefined,
        tabId,
    };
}

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
    // Every toast the pointer or the keyboard focus is on: each stays up while it is held, however many are.
    // By the toast itself, not its id: one that is removed while held (closed, capped) takes its hold with
    // it, and a notice pushed again under the same id starts unheld.
    const paused = new WeakSet<NotificationType>();
    const pauseToast = (n: NotificationType) => void paused.add(n);
    const resumeToast = (n: NotificationType) => void paused.delete(n);

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

    /** Copies the toast's title, full message and debug details (toast-debug.ts). Resolves once it is on the clipboard. */
    const copyNotification = async (id: string): Promise<void> => {
        const notif = notifications().find((n) => n.id === id);
        if (!notif) return;
        try {
            await clipboardWriteText(toastDebugText(notif, liveToastDebugEnv()));
        } catch (err) {
            console.error("Failed to copy text: ", err);
            throw err;
        }
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
            prev.filter((n) => !n.expiration || n.persistent || n.expiration > now || paused.has(n))
        );
    });

    const formatTimestamp = (timestamp: string): string => {
        const notificationTime = new Date(timestamp).getTime();
        const now = Date.now();
        return now - notificationTime < NOTIFICATION_AGO_DAYS * 86_400_000
            ? formatRelativeTime(notificationTime, now, { locale: formatLocale(), nowBelowSecs: 60 })
            : new Date(timestamp).toLocaleString();
    };

    return {
        notifications,
        hoveredId,
        setHoveredId,
        pauseToast,
        resumeToast,
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
