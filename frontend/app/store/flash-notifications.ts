// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Flash errors / notifications — split out of global.ts (see global.ts's
// "Flash errors / notifications" section for the original context).
// Re-exported from global.ts for backward-compat (97 files import from that
// module).

import { createSignal } from "solid-js";

const [flashErrors, setFlashErrors] = createSignal<FlashErrorType[]>([]);
export { flashErrors };
export const [notifications, setNotifications] = createSignal<NotificationType[]>([]);
export const [notificationPopoverMode] = createSignal(false);

export function pushFlashError(ferr: FlashErrorType) {
    if (ferr.expiration == null) ferr.expiration = Date.now() + 5000;
    ferr.id = crypto.randomUUID();
    setFlashErrors((prev) => [...prev, ferr]);
}

/** The most toasts kept at once; past it the oldest ordinary one goes first,
 *  a persistent one (it stays until closed) only when nothing else is left. */
export const MAX_KEPT_NOTIFICATIONS = 20;

function addOrUpdateNotification(notif: NotificationType) {
    setNotifications((prev) => capNotifications([...prev.filter((n) => n.id !== notif.id), notif]));
}

/** `list` trimmed to `MAX_KEPT_NOTIFICATIONS`, oldest ordinary toasts first. */
export function capNotifications(list: NotificationType[]): NotificationType[] {
    let over = list.length - MAX_KEPT_NOTIFICATIONS;
    if (over <= 0) return list;
    const kept = list.filter((n) => n.persistent || over-- <= 0);
    return kept.slice(-MAX_KEPT_NOTIFICATIONS);
}

export function pushNotification(notif: NotificationType) {
    if (!notif.id && notif.persistent) return;
    notif.id = notif.id ?? crypto.randomUUID();
    addOrUpdateNotification(notif);
}

export function removeNotificationById(id: string) {
    setNotifications((prev) => prev.filter((n) => n.id !== id));
}

export function removeFlashError(id: string) {
    setFlashErrors((prev) => prev.filter((ferr) => ferr.id !== id));
}

