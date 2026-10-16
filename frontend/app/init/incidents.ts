// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * After AgentMux recovers from something (a window's renderer crashing,
 * running out of memory or hanging; quitting because its window closed), the
 * user is told what happened and what AgentMux did to keep their work, in a
 * toast that stays until they close it
 * (docs/specs/SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.2). The host
 * keeps the record (crates/cef `commands::incidents`); closing the toast
 * acknowledges it there, which closes it in every window and keeps it from
 * coming back on the next start.
 */

import { getApi, pushNotification, removeNotificationById } from "@/store/global";

export type Incident = {
    id: string;
    at_ms: number;
    kind: string;
    what: string;
    cause?: string;
    did?: string[];
    lost?: string[];
    evidence?: string;
    acknowledged_at_ms?: number;
};

/** The toast's id for an incident, shared by every window. */
export function incidentToastId(id: string): string {
    return `incident-${id}`;
}

/** The toast's body: why (when known), what was kept, what may be lost. */
export function incidentMessage(i: Incident): string {
    const parts: string[] = [];
    if (i.cause) parts.push(i.cause);
    if (i.did?.length) parts.push(`AgentMux saved: ${i.did.join(" ")}`);
    if (i.lost?.length) parts.push(`May be lost: ${i.lost.join(" ")}`);
    return parts.join(" ");
}

export function incidentNotification(i: Incident): NotificationType {
    return {
        id: incidentToastId(i.id),
        persistent: true,
        type: "warning",
        icon: "triangle-exclamation",
        title: i.what,
        message: incidentMessage(i),
        timestamp: new Date(i.at_ms).toLocaleString(),
        onClose: () => {
            void getApi().acknowledgeIncident?.(i.id);
        },
    };
}

function show(i: Incident): void {
    if (!i?.id || i.acknowledged_at_ms) return;
    pushNotification(incidentNotification(i));
}

let started = false;

/** Show what the user hasn't closed yet, and follow new incidents and
 *  acknowledgements from other windows. Never throws: it must not hold up or
 *  break startup. */
export async function surfaceIncidents(): Promise<void> {
    if (started) return;
    started = true;
    const api = getApi();
    try {
        void api.listen<Incident>("incident-recorded", show);
        void api.listen<{ id: string }>("incident-acknowledged", (p) => {
            if (p?.id) removeNotificationById(incidentToastId(p.id));
        });
        const listed = (await api.listIncidents?.()) as { incidents?: Incident[] } | null | undefined;
        // Oldest first, so the newest ends up on top.
        for (const i of [...(listed?.incidents ?? [])].reverse()) show(i);
    } catch {
        // An older host, or no record: nothing to tell.
    }
}
