// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A hosted workspace that is going to stop (AgentMux Web: the timer, a person's stop, the credit rule). srv gets the stop time
// and a fixed reason id from the workspace's gateway (`/api/v1/host/notice`) and publishes `workspace:stop`; this keeps the
// window's copy so the status bar can say when and why. A desktop has no such host, so it never receives the event and the
// status item never shows.

import { createSignal } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { readMuxEventHistory } from "@/app/store/mps-history";
import type { WorkspaceStopView } from "@/types/rpc/WorkspaceStopView";

/** A stop that is held: when, why as a person reads it, and the id behind it. */
export interface WorkspaceStop {
    stopAtMs: number;
    reasonId: string;
    reason: string;
}

/** A stop this long past its time is not shown: the workspace is gone or the notice is stale. */
const STALE_AFTER_MS = 5 * 60_000;

/** The held stop from an event, or null when it is cleared, malformed, or long past. The reason is srv's fixed phrase, never free text. */
export function parseWorkspaceStop(view: WorkspaceStopView | null | undefined, now: number): WorkspaceStop | null {
    if (!view || view.status !== "pending") return null;
    if (!Number.isFinite(view.stop_at_ms) || view.stop_at_ms <= 0 || view.stop_at_ms + STALE_AFTER_MS < now) return null;
    return { stopAtMs: view.stop_at_ms, reasonId: String(view.reason_id ?? ""), reason: String(view.reason ?? "") };
}

/** `m:ss` to the stop, or `0:00` once it has come. */
export function countdown(stopAtMs: number, now: number): string {
    const s = Math.max(0, Math.ceil((stopAtMs - now) / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The time of day the workspace stops, as the person's clock reads it. */
export function clockTime(stopAtMs: number): string {
    return new Date(stopAtMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

const [stop, setStop] = createSignal<WorkspaceStop | null>(null);

/** The stop the workspace is going to make, if any. */
export const workspaceStop = stop;

let started = false;

/** Follow the event, and read the one held when this window opened. Started by the status item; harmless to call again. */
export function startWorkspaceStop(): void {
    if (started) return;
    started = true;
    // A live event is the truth: a replay that resolves after one must not undo it.
    let sawLive = false;
    onMuxEvent(WpsEvent.WorkspaceStop, (view) => {
        sawLive = true;
        setStop(parseWorkspaceStop(view, Date.now()));
    });
    readMuxEventHistory(WpsEvent.WorkspaceStop, "", 1)
        .then((held) => {
            if (!sawLive) setStop(parseWorkspaceStop(held[held.length - 1], Date.now()));
        })
        .catch(() => {});
}

/** For tests: forget everything. */
export function resetWorkspaceStopForTest(): void {
    started = false;
    setStop(null);
}
