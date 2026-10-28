// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sensitive operations counting down (docs/specs/SPEC_SUPER_AGENT_2026_10_25.md §5): srv publishes
 * `sensitive:pending` when the super agent approved one, and `sensitive:cleared` when the person
 * vetoed it or it ran. The events are global, so one subscription serves every pane that shows
 * the banner, and the chime plays once per countdown, not once per pane.
 */

import { createSignal, type Accessor } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { readMuxEventHistory } from "@/app/store/mps-history";
import { playShutdownPendingTone } from "@/app/notification/sound/sound-service";
import type { SensitiveView } from "@/types/rpc/SensitiveView";

/** The second chime, this long before the deadline. */
export const LAST_CALL_MS = 5_000;
/** A countdown whose end was never announced is dropped this long after its deadline. */
const STALE_MS = 5_000;

export interface SensitiveCountdown {
    request_id: string;
    op: string;
    /** What will happen, built by srv from the request's facts. */
    summary: string;
    requester: string;
    /** The super agent that approved it; empty when none reviewed it. */
    approver: string;
    deadline_ms: number;
}

/** A countdown still running, from a `sensitive:pending` payload. */
export function parseCountdown(data: unknown): SensitiveCountdown | null {
    const d = data as Partial<SensitiveView> | null;
    if (!d || typeof d.request_id !== "string" || typeof d.deadline_ms !== "number") return null;
    if (d.status != null && d.status !== "pending") return null;
    return {
        request_id: d.request_id,
        op: typeof d.op === "string" ? d.op : "",
        summary: typeof d.summary === "string" ? d.summary : "",
        requester: typeof d.requester === "string" ? d.requester : "",
        approver: typeof d.approver === "string" ? d.approver : "",
        deadline_ms: d.deadline_ms,
    };
}

export function withCountdown(list: SensitiveCountdown[], c: SensitiveCountdown): SensitiveCountdown[] {
    return [...list.filter((x) => x.request_id !== c.request_id), c];
}

export function withoutCountdown(list: SensitiveCountdown[], requestId: string): SensitiveCountdown[] {
    return list.filter((x) => x.request_id !== requestId);
}

/**
 * What a pane that opens mid-countdown should show, from the persisted last `pending` and
 * last `cleared` events: the pending one, unless it was cleared or its deadline has passed.
 */
export function replayCountdown(pending: unknown, cleared: unknown, now: number): SensitiveCountdown | null {
    const p = parseCountdown(pending);
    if (!p || p.deadline_ms <= now) return null;
    const clearedId = (cleared as { request_id?: unknown } | null)?.request_id;
    return clearedId === p.request_id ? null : p;
}

/** Whole seconds left, never negative. */
export function secondsLeft(c: SensitiveCountdown, now: number): number {
    return Math.max(0, Math.ceil((c.deadline_ms - now) / 1000));
}

/** "Parlo asked: install a widget (…). Approved by Y. Starts in 12 s." The words come from srv. */
export function countdownText(c: SensitiveCountdown, seconds: number): string {
    const who = c.requester || "An agent";
    const review = c.approver ? `Approved by ${c.approver}.` : "No super agent reviewed this.";
    return `${who} asked: ${c.summary}. ${review} Starts in ${seconds} s.`;
}

const [open, setOpen] = createSignal<SensitiveCountdown[]>([]);
let users = 0;
let stop: (() => void) | null = null;

function begin(): () => void {
    // Set by any live event: from then on it, not the history replay, is the truth.
    let sawLive = false;
    const lastCalled = new Set<string>();
    const unsubPending = onMuxEvent(WpsEvent.SensitivePending, (view) => {
        sawLive = true;
        const c = parseCountdown(view);
        if (!c) return;
        setOpen((list) => withCountdown(list, c));
        playShutdownPendingTone();
    });
    const unsubCleared = onMuxEvent(WpsEvent.SensitiveCleared, (cleared) => {
        sawLive = true;
        if (typeof cleared.request_id === "string") setOpen((list) => withoutCountdown(list, cleared.request_id));
    });
    const last = <T,>(items: T[]): T | null => items[items.length - 1] ?? null;
    void Promise.all([
        readMuxEventHistory(WpsEvent.SensitivePending, "", 1).then(last).catch(() => null),
        readMuxEventHistory(WpsEvent.SensitiveCleared, "", 1).then(last).catch(() => null),
    ]).then(([p, cleared]) => {
        if (sawLive) return;
        const replayed = replayCountdown(p, cleared, Date.now());
        if (replayed) setOpen((list) => withCountdown(list, replayed));
    });
    const timer = setInterval(() => {
        const now = Date.now();
        for (const c of open()) {
            if (c.deadline_ms - now <= LAST_CALL_MS && c.deadline_ms > now && !lastCalled.has(c.request_id)) {
                lastCalled.add(c.request_id);
                playShutdownPendingTone();
            }
        }
        setOpen((list) => (list.some((c) => now - c.deadline_ms > STALE_MS) ? list.filter((c) => now - c.deadline_ms <= STALE_MS) : list));
    }, 250);
    return () => {
        unsubPending();
        unsubCleared();
        clearInterval(timer);
        setOpen([]);
    };
}

/**
 * The countdowns open now. Call once per component that shows them; the returned `release`
 * goes in its cleanup. The subscription lives while any component holds it.
 */
export function useSensitiveCountdowns(): { open: Accessor<SensitiveCountdown[]>; release: () => void } {
    if (users++ === 0) stop = begin();
    let released = false;
    return {
        open,
        release: () => {
            if (released) return;
            released = true;
            if (--users === 0) {
                stop?.();
                stop = null;
            }
        },
    };
}

/** Forget a countdown the person just decided (or that srv says is too late). */
export function dropCountdown(requestId: string): void {
    setOpen((list) => withoutCountdown(list, requestId));
}
