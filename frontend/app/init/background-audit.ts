// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Background-service audit surfacing — issue #2977 Workstream 4,
// SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md §6.
//
// The host records what happened while AgentMux was running with no window
// open (see agentmux-cef/src/background_audit.rs). §6 requires that record be
// "surfaced the next time a window (or the tray panel) opens" — an audit log
// nobody is shown is not an audit log, which is the whole point of the Zoom
// 2019 / Recall 2024 precedents the spec cites.
//
// This is that half: called once during window init, it drains whatever the
// user has not already been shown and raises a notification.

import { pushNotification } from "@/store/flash-notifications";

type AuditEntry = { at_ms: number; kind: string };
type AuditPayload = { entries?: AuditEntry[]; unattended?: boolean };

/** Human-readable summary of an unattended stretch. */
export function summarize(entries: AuditEntry[]): string | null {
    // Pair up went_unattended -> observed. A trailing went_unattended with no
    // matching observed is the CURRENT period (this window is what ends it),
    // so it counts too.
    const periods = entries.filter((e) => e.kind === "went_unattended").length;
    if (periods === 0) return null;

    const first = entries.find((e) => e.kind === "went_unattended");
    const since = first ? new Date(first.at_ms).toLocaleString() : null;

    if (periods === 1) {
        return since
            ? `AgentMux kept running in the background since ${since}.`
            : "AgentMux kept running in the background while no window was open.";
    }
    return since
        ? `AgentMux ran in the background ${periods} times since ${since}.`
        : `AgentMux ran in the background ${periods} times while no window was open.`;
}

/**
 * Drain and show the background-service audit record.
 *
 * Deliberately never throws: this runs during window init, and a failure to
 * show an informational notice must not be able to break startup. A host that
 * predates the IPC command simply returns an error, which is swallowed.
 */
export async function surfaceBackgroundAudit(): Promise<void> {
    try {
        const { invokeCommand } = await import("@/app/platform/ipc");
        const payload = (await invokeCommand("background_audit_take", {})) as AuditPayload | null;
        const entries = payload?.entries ?? [];
        const message = summarize(entries);
        if (!message) return;

        pushNotification({
            icon: "moon",
            title: "Ran while you were away",
            message,
            timestamp: new Date().toLocaleString(),
            type: "info",
        });
    } catch {
        // Older host, IPC unavailable, or no audit log — nothing to surface.
    }
}
