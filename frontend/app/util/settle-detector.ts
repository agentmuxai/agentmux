// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * scheduleOnSettle — call `onSettled` once the main thread has gone quiet
 * (no Long Tasks) for `settleMs`, or `maxMs` has elapsed, whichever comes
 * first. Same detection algorithm as `frontend/app/store/tab-reveal.ts`'s
 * `scheduleRevealLift` (issue #774,
 * `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`), factored out so a caller
 * can run its own independent instance — tab-reveal.ts's version is
 * module-level singleton state (one gate, keyed to nothing), which is right
 * for "the current tab switch" but wrong for "N agent panes each waiting on
 * their own settle," since concurrent callers would clobber each other's
 * detector.
 *
 * A flat `setTimeout` is the wrong tool here: the cost this is meant to
 * cover (virtualizer measureElement, markdown render, ResizeObserver
 * fan-out — see SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md) happens
 * *after* the triggering dispatch and scales with content size, so a fixed
 * delay either fires too early for heavy sessions (revealing unpainted
 * content) or wastes time on light ones. Watching for actual Long-Task
 * quiet adapts to both.
 *
 * Returns a cancel function — call it if the caller unmounts/disposes
 * before settling (e.g. the pane closes mid-load).
 */
export function scheduleOnSettle(
    onSettled: () => void,
    opts?: { settleMs?: number; maxMs?: number },
): () => void {
    const settleMs = opts?.settleMs ?? 80;
    const maxMs = opts?.maxMs ?? 800;
    const startedAt = performance.now();
    let lastBusyAt = startedAt;
    let cancelled = false;
    let observer: PerformanceObserver | null = null;
    let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

    const cancel = (): void => {
        cancelled = true;
        observer?.disconnect();
        observer = null;
        if (fallbackTimer !== null) {
            clearTimeout(fallbackTimer);
            fallbackTimer = null;
        }
    };

    const fallbackToHardCap = (): void => {
        // No longtask data available (no PerformanceObserver, or the
        // "longtask" entry type isn't supported — historically Safari) —
        // wait the full maxMs budget rather than settleMs, since without
        // longtask signal there's no way to detect the real settle moment.
        fallbackTimer = setTimeout(() => {
            fallbackTimer = null;
            if (!cancelled) onSettled();
        }, maxMs);
    };

    if (typeof PerformanceObserver === "undefined") {
        fallbackToHardCap();
        return cancel;
    }

    try {
        observer = new PerformanceObserver((entries) => {
            for (const e of entries.getEntries()) {
                if (e.duration > 50) lastBusyAt = performance.now();
            }
        });
        observer.observe({ entryTypes: ["longtask"] });
    } catch {
        fallbackToHardCap();
        return cancel;
    }

    const tick = (): void => {
        if (cancelled) return;
        const now = performance.now();
        const settledSinceLastBusy = now - lastBusyAt >= settleMs;
        const hardCapHit = now - startedAt >= maxMs;
        if (settledSinceLastBusy || hardCapHit) {
            cancel();
            onSettled();
            return;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);

    return cancel;
}
