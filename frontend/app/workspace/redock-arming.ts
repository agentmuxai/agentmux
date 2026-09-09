// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Redock arming state machine — the single decision point for "has this drag
 * become a dock attempt?".
 *
 * Previously this logic was hand-implemented twice (once for the Windows
 * native-move-loop path, once for the macOS/Linux JS-driven path) across ~20
 * mutable closure variables, with each copy accumulating its own fallbacks.
 * `docs/retro/retro-redock-ghost-landing-reliability-2026-07-27.md` flagged the
 * duplication; `docs/specs/SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` §5.4
 * is the extraction.
 *
 * The module never reads a clock. Callers supply `t` from their own monotonic
 * source, which is what makes the whole gate unit-testable.
 *
 * DESIGN NOTE — why there is no "events went quiet" fallback here:
 * the previous implementation inferred dwell from the *absence* of hover
 * events, because the Win32 drag loop only emitted from `WM_MOUSEMOVE` and so
 * fell silent exactly when the cursor held still. That inference armed a redock
 * even after the velocity gate had explicitly rejected the entry, which is the
 * bug the spec above exists to fix. Dwell is now measured only from samples
 * that actually confirmed a target at a qualifying speed; the host supplies a
 * heartbeat (`agentmux-cef/src/ui_tasks/drag.rs`, `DRAG_TICK_ID`) so those
 * samples keep arriving while stationary. Do not reintroduce a timeout-based
 * arming path here.
 */

import { REDOCK_DWELL_MS, REDOCK_VELOCITY_PX_PER_S } from "./floating-pane-constants";

export interface RedockHoverSample {
    /** Resolved redock target window label, or null when over nothing dockable. */
    target: string | null;
    /**
     * Cursor position in CSS px. Callers normalise from their own space.
     *
     * Omit both when the event carries no cursor — the `clear_floating_redock_hover`
     * teardown broadcast is target-only. The module then reuses its last known
     * position so the sample reads as "no motion" instead of a jump from the
     * origin, which would otherwise trip the velocity gate on the NEXT real
     * sample and cost an extra heartbeat before arming.
     */
    x?: number;
    y?: number;
    /** Monotonic timestamp in ms (`performance.now()`). */
    t: number;
}

export interface RedockArmingOutcome {
    /** Whether the gesture is now a committed dock attempt. */
    armed: boolean;
    /** True when this sample invalidated an indicator that was showing. */
    clearIndicator: boolean;
}

export interface RedockArming {
    sample(s: RedockHoverSample): RedockArmingOutcome;
    /**
     * Arming state as of `now`. Re-checks the dwell against the wall clock so a
     * mouseup landing between two samples still sees a dwell that has in fact
     * elapsed. This is a recheck of a real confirmed-hover timestamp, not an
     * inference from missing samples — see the design note above.
     */
    isArmed(now: number): boolean;
    /** Current resolved target, for pre-capturing the ghost before teardown. */
    target(): string | null;
    reset(): void;
}

export function createRedockArming(opts?: {
    dwellMs?: number;
    velocityPxPerS?: number;
}): RedockArming {
    const dwellMs = opts?.dwellMs ?? REDOCK_DWELL_MS;
    const velocityPxPerS = opts?.velocityPxPerS ?? REDOCK_VELOCITY_PX_PER_S;

    let target: string | null = null;
    // null = the dwell clock is not running. Set to the timestamp at which the
    // cursor was first confirmed over `target` at a qualifying speed.
    let firstSeenAt: number | null = null;
    let armed = false;
    let lastT: number | null = null;
    let lastX = 0;
    let lastY = 0;

    const dwellServed = (now: number): boolean =>
        target !== null && firstSeenAt !== null && now - firstSeenAt >= dwellMs;

    return {
        sample(s: RedockHoverSample): RedockArmingOutcome {
            const wasArmed = armed;

            // Velocity is measured between consecutive samples. A zero (or
            // backwards) interval carries no speed information — treat it as no
            // motion rather than dividing by zero into a spurious rejection.
            const x = s.x ?? lastX;
            const y = s.y ?? lastY;
            let tooFast = false;
            if (lastT !== null && s.t > lastT) {
                const dx = x - lastX;
                const dy = y - lastY;
                const velocity = Math.sqrt(dx * dx + dy * dy) / ((s.t - lastT) / 1000);
                tooFast = velocity > velocityPxPerS;
            }
            lastT = s.t;
            lastX = x;
            lastY = y;

            if (tooFast) {
                // Keep the target identity but stop the clock: if the cursor
                // settles on this same window the dwell restarts from the
                // moment it actually slowed, not from when it arrived.
                target = s.target;
                firstSeenAt = null;
                armed = false;
            } else if (s.target === null) {
                target = null;
                firstSeenAt = null;
                armed = false;
            } else if (s.target !== target) {
                target = s.target;
                firstSeenAt = s.t;
                armed = false;
            } else if (firstSeenAt === null) {
                firstSeenAt = s.t;
            } else if (s.t - firstSeenAt >= dwellMs) {
                armed = true;
            }

            return { armed, clearIndicator: wasArmed && !armed };
        },

        isArmed(now: number): boolean {
            return armed || dwellServed(now);
        },

        target(): string | null {
            return target;
        },

        reset(): void {
            target = null;
            firstSeenAt = null;
            armed = false;
            lastT = null;
            lastX = 0;
            lastY = 0;
        },
    };
}
