// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How the Audit pane stays current without polling every few seconds: srv says `audit:changed` when it writes a
// row (at most ten times a second), the pane asks `audit.query` again, and a slow timer and a reconnect catch what
// no event told it (agent starts and ends come from the transcript store, which does not say; an event sent while
// the socket was down is gone). docs/specs/SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.1.

import { createEffect, onCleanup } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { onReconnectRefresh } from "@/app/store/domain-store";

/** How often the pane asks again while shown, for the rows no event announces. */
export const AUDIT_SAFETY_POLL_MS = 30_000;

/** A busy agent makes ten events a second; the pane asks again at most this often for them. */
export const AUDIT_EVENT_THROTTLE_MS = 500;

/** `refresh` when the pane is shown, when srv writes a row, after a reconnect, and on the slow timer. */
export function useAuditLive(visible: () => boolean, refresh: () => void): void {
    createEffect(() => {
        if (!visible()) return;
        refresh();
        let waiting: number | undefined;
        const stopEvent = onMuxEvent(WpsEvent.AuditChanged, () => {
            if (waiting !== undefined) return;
            waiting = window.setTimeout(() => {
                waiting = undefined;
                refresh();
            }, AUDIT_EVENT_THROTTLE_MS);
        });
        const stopReconnect = onReconnectRefresh(refresh);
        const timer = window.setInterval(refresh, AUDIT_SAFETY_POLL_MS);
        onCleanup(() => {
            stopEvent();
            stopReconnect();
            window.clearInterval(timer);
            if (waiting !== undefined) window.clearTimeout(waiting);
        });
    });
}

/**
 * `run` one at a time: a call while it runs makes it run once more when it ends (not once per call), so a burst
 * of events is one request and an event during a request is not lost.
 */
export function serial(run: () => Promise<void>): () => Promise<void> {
    let running: Promise<void> | null = null;
    let again = false;
    return async () => {
        if (running) {
            again = true;
            return running;
        }
        running = (async () => {
            try {
                do {
                    again = false;
                    await run();
                } while (again);
            } finally {
                running = null;
            }
        })();
        return running;
    };
}
