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
export const [notificationPopoverMode, setNotificationPopoverMode] = createSignal(false);

export function pushFlashError(ferr: FlashErrorType) {
    if (ferr.expiration == null) ferr.expiration = Date.now() + 5000;
    ferr.id = crypto.randomUUID();
    setFlashErrors((prev) => [...prev, ferr]);
}

function addOrUpdateNotification(notif: NotificationType) {
    setNotifications((prev) => {
        const withoutThis = prev.filter((n) => n.id !== notif.id);
        return [...withoutThis, notif];
    });
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

