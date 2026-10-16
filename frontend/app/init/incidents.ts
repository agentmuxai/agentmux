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
import { createSignal } from "solid-js";

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
    // Each `did` line is a whole sentence saying what AgentMux did or kept.
    if (i.did?.length) parts.push(i.did.join(" "));
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

/** How many incidents the status bar's history keeps (§4.3). */
export const HISTORY_MAX = 20;

const [history, setHistory] = createSignal<Incident[]>([]);

/** The newest incidents, newest first, as this window knows them: the
 *  status bar's "Recent incidents" (§4.3). */
export const recentIncidents = history;

/** Whether any notice is still open: the uptime's marker (§4.3). */
export const hasOpenIncident = (): boolean => history().some((i) => !i.acknowledged_at_ms);

function remember(i: Incident): void {
    setHistory((h) => [i, ...h.filter((x) => x.id !== i.id)].slice(0, HISTORY_MAX));
}

function acknowledged(id: string): void {
    setHistory((h) => h.map((x) => (x.id === id && !x.acknowledged_at_ms ? { ...x, acknowledged_at_ms: Date.now() } : x)));
}

function show(i: Incident): void {
    if (!i?.id || i.acknowledged_at_ms) return;
    pushNotification(incidentNotification(i));
}

/** An incident as text for a bug report: everything the record says. */
export function incidentReport(i: Incident): string {
    const lines = [`AgentMux incident ${i.id} (${i.kind})`, `When: ${new Date(i.at_ms).toISOString()}`, `What: ${i.what}`];
    if (i.cause) lines.push(`Why: ${i.cause}`);
    for (const d of i.did ?? []) lines.push(`Did: ${d}`);
    for (const l of i.lost ?? []) lines.push(`May be lost: ${l}`);
    if (i.evidence) lines.push(`Evidence: ${i.evidence}`);
    return lines.join("\n");
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
        void api.listen<Incident>("incident-recorded", (i) => {
            if (!i?.id) return;
            remember(i);
            show(i);
        });
        void api.listen<{ id: string }>("incident-acknowledged", (p) => {
            if (!p?.id) return;
            acknowledged(p.id);
            removeNotificationById(incidentToastId(p.id));
        });
        const listed = (await api.listIncidents?.()) as { incidents?: Incident[] } | null | undefined;
        const incidents = (listed?.incidents ?? []).filter((i) => i?.id);
        // Keep what the events brought while the list was on its way.
        setHistory((live) => {
            const ids = new Set(live.map((i) => i.id));
            return [...live, ...incidents.filter((i) => !ids.has(i.id))].sort((a, b) => b.at_ms - a.at_ms).slice(0, HISTORY_MAX);
        });
        // Oldest first, so the newest ends up on top.
        for (const i of [...incidents].reverse()) show(i);
    } catch {
        // An older host, or no record: nothing to tell.
    }
}
