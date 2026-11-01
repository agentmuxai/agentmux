// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, type Accessor, type JSX } from "solid-js";
import { liveRole } from "./toast-live";
import "./toast-announcer.scss";

/** What a screen reader hears of a toast: its title, then its message. */
export function announcementOf(n: Pick<NotificationType, "title" | "message">): string {
    return [n.title, n.message].map((t) => t?.trim()).filter(Boolean).join(". ");
}

/**
 * The live regions toasts are announced through. They are mounted before any toast is, and a toast's words
 * are put into one when it arrives: a status region that enters the page already holding its text is not
 * reliably announced (WCAG technique ARIA22). Nothing here takes focus, and the toasts themselves are plain
 * content, so a toast is not read twice.
 */
export function ToastAnnouncer(props: { notifications: Accessor<NotificationType[]> }): JSX.Element {
    const [polite, setPolite] = createSignal("");
    const [urgent, setUrgent] = createSignal("");
    // The toasts already there when this mounted, and each one announced since.
    const seen = new Set<string>(props.notifications().flatMap((n) => (n.id ? [n.id] : [])));
    createEffect(() => {
        const heard: string[] = [];
        const alerts: string[] = [];
        for (const n of props.notifications()) {
            if (!n.id || n.hidden || seen.has(n.id)) continue;
            seen.add(n.id);
            (liveRole(n) === "alert" ? alerts : heard).push(announcementOf(n));
        }
        if (heard.length) setPolite(heard.join(" "));
        if (alerts.length) setUrgent(alerts.join(" "));
    });
    return (
        <>
            <div class="toast-announcer" role="status" aria-live="polite" aria-atomic="true">
                {polite()}
            </div>
            <div class="toast-announcer" role="alert" aria-live="assertive" aria-atomic="true">
                {urgent()}
            </div>
        </>
    );
}
