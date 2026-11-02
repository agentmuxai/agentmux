// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, type Accessor, type JSX } from "solid-js";
import { liveRole } from "./toast-live";
import "./toast-announcer.scss";

/** What a screen reader hears of a toast: its title, then its message. */
export function announcementOf(n: Pick<NotificationType, "title" | "message">): string {
    return [n.title, n.message].map((t) => t?.trim()).filter(Boolean).join(". ");
}

const ZERO_WIDTH = "\u200B";

/** A region's next text: the words, made different from what it holds when they are the same, so they are read again. */
export function nextAnnouncement(current: string, words: string): string {
    return current === words ? words + ZERO_WIDTH : words;
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
    // Each toast announced, by id: the very object announced, so a toast pushed again under the same id
    // (re-raised after it went, or updated in place) is a new one. The toasts there at mount are not announced.
    const heard = new Map<string, NotificationType>(props.notifications().flatMap((n) => (n.id ? [[n.id, n] as const] : [])));
    createEffect(() => {
        const now = new Map<string, NotificationType>();
        const calm: string[] = [];
        const alerts: string[] = [];
        for (const n of props.notifications()) {
            if (!n.id) continue;
            now.set(n.id, n);
            if (n.hidden || heard.get(n.id) === n) continue;
            (liveRole(n) === "alert" ? alerts : calm).push(announcementOf(n));
        }
        // What went is forgotten, so it is announced again if it comes back.
        heard.clear();
        for (const [id, n] of now) heard.set(id, n);
        if (calm.length) setPolite((cur) => nextAnnouncement(cur, calm.join(" ")));
        if (alerts.length) setUrgent((cur) => nextAnnouncement(cur, alerts.join(" ")));
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
