// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Performance health signals for this window: low RAM, low page file, slow
 * backend. The host pushes each change as a `health-signal` event and keeps
 * the active ones, which a window reads once when it starts so it can catch
 * up on an episode already under way. The status bar's backend dot and its
 * panel read this store. REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08.
 *
 * Drive it from DevTools:
 *   window.dispatchEvent(new CustomEvent('agentmux-event',
 *     { detail: { event: 'health-signal', payload: { kind: 'backend', level: 'warn', avg_ms: 1800 } } }))
 */

import { getApi } from "@/app/store/app-api";
import { createSignal } from "solid-js";

export type HealthKind = "ram" | "pagefile" | "backend";
export type HealthLevel = "normal" | "warn" | "critical";
export type ActiveHealthLevel = Exclude<HealthLevel, "normal">;

export interface HealthPayload {
    kind: HealthKind;
    level: HealthLevel;
    /** When the episode began (ms epoch), set by the host. */
    since_ms?: number;
    // "ram"
    phys_free_mb?: number;
    // "pagefile"
    commit_free_mb?: number;
    system_managed?: boolean;
    disk_free_pct?: number;
    // "backend"
    avg_ms?: number;
}

export interface HealthSignal {
    kind: HealthKind;
    level: ActiveHealthLevel;
    since: number;
    payload: HealthPayload;
}

const KINDS: HealthKind[] = ["backend", "pagefile", "ram"];

export function severity(level: HealthLevel): number {
    return level === "critical" ? 2 : level === "warn" ? 1 : 0;
}

/** Fold one event into the active signals: worst first, then a fixed order
 *  of kinds. A normal level removes its kind; returns `signals` itself when
 *  nothing changed. */
export function applyHealthEvent(signals: HealthSignal[], p: HealthPayload, now: number): HealthSignal[] {
    if (!p || !KINDS.includes(p.kind)) return signals;
    const prev = signals.find((s) => s.kind === p.kind);
    if (p.level !== "warn" && p.level !== "critical") {
        return prev ? signals.filter((s) => s !== prev) : signals;
    }
    const next: HealthSignal = { kind: p.kind, level: p.level, since: p.since_ms ?? prev?.since ?? now, payload: p };
    return [...signals.filter((s) => s !== prev), next].sort(
        (a, b) => severity(b.level) - severity(a.level) || KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind)
    );
}

export function worstLevel(signals: HealthSignal[]): HealthLevel {
    return signals[0]?.level ?? "normal";
}

/** Fold the start-up snapshot in, skipping every kind an event has already
 *  reported: the event is newer, and a snapshot read before a recovery must
 *  not bring the signal back. */
export function applySnapshot(
    signals: HealthSignal[],
    snapshot: HealthPayload[],
    heard: ReadonlySet<HealthKind>,
    now: number
): HealthSignal[] {
    return snapshot.filter((p) => !heard.has(p?.kind)).reduce((acc, p) => applyHealthEvent(acc, p, now), signals);
}

const [signals, setSignals] = createSignal<HealthSignal[]>([]);

/** The active signals, worst first; empty when all is normal. */
export const healthSignals = signals;
export const worstHealthLevel = (): HealthLevel => worstLevel(signals());

let stop: (() => void) | undefined;

/** Start following the host's signals; the returned function stops. Safe to
 *  call again while running (the second call is a no-op). */
export function startHealthSignals(): () => void {
    if (stop) return () => {};
    const heard = new Set<HealthKind>();
    let unlisten: (() => void) | undefined;
    let stopped = false;
    const api = getApi();
    void api
        .listen<HealthPayload>("health-signal", (p) => {
            if (p?.kind) heard.add(p.kind);
            setSignals((s) => applyHealthEvent(s, p, Date.now()));
        })
        .then((u) => {
            if (stopped) u();
            else unlisten = u;
        });
    void api
        .getHealthSignals?.()
        .then((snapshot) => {
            if (!stopped && Array.isArray(snapshot)) {
                setSignals((s) => applySnapshot(s, snapshot, heard, Date.now()));
            }
        })
        .catch(() => {});
    stop = () => {
        stopped = true;
        unlisten?.();
        stop = undefined;
    };
    return stop;
}
