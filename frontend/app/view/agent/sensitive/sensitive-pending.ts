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
/** How many of each event srv keeps for a pane that opens mid-countdown. */
const REPLAY_EVENTS = 32;
/** A countdown whose end was never announced is dropped this long after its deadline. */
const STALE_MS = 5_000;
/** A countdown that began longer ago than this was replayed to a window that connected later, not just started. */
const FRESH_MS = 5_000;
/** A countdown's first chime waits this long, so a replayed one that was already cleared stays silent. */
export const CHIME_DELAY_MS = 300;
/** How long a live clear is remembered. */
const KEEP_CLEARED_MS = 10 * 60 * 1000;

export interface SensitiveCountdown {
    request_id: string;
    op: string;
    /** What will happen, built by srv from the request's facts. */
    summary: string;
    requester: string;
    /** The super agent that approved it; empty when none reviewed it. */
    approver: string;
    deadline_ms: number;
    /** When srv began the countdown; 0 when it did not say. */
    opened_ms: number;
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
        opened_ms: typeof d.opened_ms === "number" ? d.opened_ms : 0,
    };
}

export function withCountdown(list: SensitiveCountdown[], c: SensitiveCountdown): SensitiveCountdown[] {
    return [...list.filter((x) => x.request_id !== c.request_id), c];
}

export function withoutCountdown(list: SensitiveCountdown[], requestId: string): SensitiveCountdown[] {
    return list.filter((x) => x.request_id !== requestId);
}

/**
 * What a pane that opens mid-countdown should show, from the persisted `pending` and `cleared`
 * events: every pending one not cleared since and not past its deadline. Several countdowns can
 * overlap, so this is a set, not the latest event.
 */
export function replayCountdowns(pending: unknown[], cleared: unknown[], now: number): SensitiveCountdown[] {
    const clearedIds = new Set(cleared.map((c) => (c as { request_id?: unknown } | null)?.request_id));
    let list: SensitiveCountdown[] = [];
    for (const p of pending) {
        const c = parseCountdown(p);
        if (c && c.deadline_ms > now && !clearedIds.has(c.request_id)) list = withCountdown(list, c);
    }
    return list;
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
    // Cleared by a live event, and when: a history replay that lands later must not bring these
    // back. srv keeps a finished countdown ten minutes, so no id is remembered longer.
    const clearedLive = new Map<string, number>();
    const lastCalled = new Set<string>();
    // Set when the last pane lets go: a history read still on its way must not touch the store then.
    let disposed = false;
    const chimes = new Set<ReturnType<typeof setTimeout>>();
    // The broker replays its kept events through these same handlers whenever this connection
    // subscribes: an expired one is ignored, only a countdown that has just begun chimes, and the
    // chime waits a moment so one cleared right behind it never sounds.
    const unsubPending = onMuxEvent(WpsEvent.SensitivePending, (view) => {
        const c = parseCountdown(view);
        if (!c || c.deadline_ms <= Date.now() || clearedLive.has(c.request_id)) return;
        const known = open().some((x) => x.request_id === c.request_id);
        setOpen((list) => withCountdown(list, c));
        if (known || Date.now() - c.opened_ms > FRESH_MS) return;
        const t = setTimeout(() => {
            chimes.delete(t);
            if (open().some((x) => x.request_id === c.request_id)) playShutdownPendingTone();
        }, CHIME_DELAY_MS);
        chimes.add(t);
    });
    const unsubCleared = onMuxEvent(WpsEvent.SensitiveCleared, (cleared) => {
        if (typeof cleared.request_id !== "string") return;
        clearedLive.set(cleared.request_id, Date.now());
        lastCalled.delete(cleared.request_id);
        setOpen((list) => withoutCountdown(list, cleared.request_id));
    });
    void Promise.all([
        readMuxEventHistory(WpsEvent.SensitivePending, "", REPLAY_EVENTS).catch(() => []),
        readMuxEventHistory(WpsEvent.SensitiveCleared, "", REPLAY_EVENTS).catch(() => []),
    ]).then(([pending, cleared]) => {
        if (disposed) return;
        // Merged by request: what the live events already said stands, the rest is added.
        const replayed = replayCountdowns(pending, cleared, Date.now()).filter((c) => !clearedLive.has(c.request_id));
        setOpen((list) => replayed.reduce((l, c) => (l.some((x) => x.request_id === c.request_id) ? l : withCountdown(l, c)), list));
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
        for (const [id, at] of clearedLive) if (now - at > KEEP_CLEARED_MS) clearedLive.delete(id);
        for (const id of lastCalled) if (!open().some((c) => c.request_id === id)) lastCalled.delete(id);
    }, 250);
    return () => {
        unsubPending();
        unsubCleared();
        clearInterval(timer);
        chimes.forEach(clearTimeout);
        disposed = true;
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
