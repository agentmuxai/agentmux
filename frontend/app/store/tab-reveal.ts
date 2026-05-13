// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tab-content reveal gate (issue #774, spec
// `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`).
//
// Hides the active tab's content under `visibility: hidden` while a
// fresh switch or open is settling, then reveals it once a window of
// "clean" frames (no Long Tasks > 50ms) has passed — or the hard cap
// trips. The user perceives an atomic before/after transition instead
// of the prior 3–5-stage piecemeal mount cascade.
//
// Triggered from `setActiveTab` and `createTab` in `global.ts`. The
// `tabSwitching` signal here drives a CSS class on the tab content
// root in `workspace.tsx`.
//
// Reduced-motion behaviour: the reveal is unanimated regardless, so
// `prefers-reduced-motion` users get the same behaviour. No special
// handling needed.

import { createSignal } from "solid-js";

const [tabSwitching, setTabSwitching] = createSignal(false);

export { tabSwitching };

/** Hard cap on how long the gate stays up. Past this, content reveals
 *  even if the long-task stream hasn't gone quiet — protects against
 *  perma-busy tabs (streaming agent, etc.) holding the gate open. */
const MAX_GATE_MS = 800;

/** A window of clean frames (no long tasks beyond `LONG_TASK_THRESHOLD_MS`)
 *  of at least this duration counts as "settled". 80 ms is ~5 frames at
 *  60 Hz — empirically enough to cover the bulk of post-mount measurement
 *  reflow without being noticeably slow. */
const SETTLE_MS = 80;

/** Any task at least this long is treated as "busy" — the settle clock
 *  restarts when one fires. Matches PerformanceObserver's default
 *  longtask threshold; calling it out so we can tune from one place. */
const LONG_TASK_THRESHOLD_MS = 50;

let activeObserver: PerformanceObserver | null = null;
let activeStartedAt = 0;
// Handle of the most recent fallback timer (PerformanceObserver
// unavailable / longtask unsupported). Cleared + cancelled on re-entry
// so a stale timer can't fire `setTabSwitching(false)` against a
// newer gate that's still mid-settle — same class of bug as the
// `tick()` superseded check, but for the no-observer path.
let activeFallbackTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Mark the gate active and start watching for clean frames. Idempotent —
 * a second call before the first completes resets the detector. That's
 * what handles rapid Ctrl-Tab spam.
 */
export function scheduleRevealLift(): void {
    setTabSwitching(true);

    activeStartedAt = performance.now();
    let lastLongTaskAt = activeStartedAt;

    // Tear down any pending detector from a prior switch — its
    // observer is still subscribed, and its tick() would race the new
    // one to disconnect/lift. Same applies to the fallback-timer path.
    activeObserver?.disconnect();
    activeObserver = null;
    if (activeFallbackTimer !== null) {
        clearTimeout(activeFallbackTimer);
        activeFallbackTimer = null;
    }

    if (typeof PerformanceObserver === "undefined") {
        // No PerformanceObserver in this runtime (test env, etc.).
        // Fall back to the hard cap so we still lift eventually.
        activeFallbackTimer = setTimeout(() => {
            activeFallbackTimer = null;
            setTabSwitching(false);
        }, SETTLE_MS);
        return;
    }

    let observer: PerformanceObserver;
    try {
        observer = new PerformanceObserver((entries) => {
            for (const e of entries.getEntries()) {
                if (e.duration > LONG_TASK_THRESHOLD_MS) {
                    lastLongTaskAt = performance.now();
                }
            }
        });
        observer.observe({ entryTypes: ["longtask"] });
        activeObserver = observer;
    } catch {
        // longtask observer not supported (Safari historically). Fall
        // back to fixed SETTLE_MS.
        activeFallbackTimer = setTimeout(() => {
            activeFallbackTimer = null;
            setTabSwitching(false);
        }, SETTLE_MS);
        return;
    }

    // Hold the start timestamp in the closure so rapid Ctrl-Tab can't
    // make this tick read a NEWER `activeStartedAt` for hard-cap calc.
    const startedAt = activeStartedAt;

    const tick = () => {
        // Identity check against the captured observer — the
        // module-level `activeObserver` may now point to a newer
        // observer that a subsequent `scheduleRevealLift()` installed.
        // If so, the older tick must NOT touch the signal: the newer
        // detector owns it. The earlier `activeObserver == null` check
        // didn't catch this because re-entry sets it to the new
        // observer before the old tick observes the swap.
        if (activeObserver !== observer) return;

        const now = performance.now();
        const settledSinceLastBusy = now - lastLongTaskAt >= SETTLE_MS;
        const hardCapHit = now - startedAt >= MAX_GATE_MS;

        if (settledSinceLastBusy || hardCapHit) {
            observer.disconnect();
            activeObserver = null;
            setTabSwitching(false);
            return;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
}
