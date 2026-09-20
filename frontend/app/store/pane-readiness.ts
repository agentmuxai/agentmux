// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneReadiness — one authority per pane for "is this still assembling?".
 *
 * Phase 1 of `docs/specs/SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md`. Today a
 * single agent-pane mount can stack up to four independent loading indicators
 * (block-level `ready()` spinner, block `<Suspense>` fallback, agent-view's
 * `.agent-pane-loading-overlay`, and AgentPicker's own render of that SAME class),
 * each with its own fade duration and unmount timer, none aware of the others —
 * two were measured on screen simultaneously. There is no moment defined as "the
 * pane may now appear", only several components independently deciding to stop
 * hiding their own part.
 *
 * This module is that moment.
 *
 * ## Stated, not inferred
 *
 * `agent-view.tsx` currently reveals via a six-stage proxy chain: historyPainted →
 * authPhaseSettled → a Long-Task quiet detector → 2×rAF → a CSS fade → a 220ms
 * timer. Each stage has a real reason, but none of them is an actual statement
 * from the thing being waited on, so any one firing early reveals a half-assembled
 * pane. Here, a dependency says when it is done:
 *
 *     const historyDone = readiness.gate("history");
 *     // ...later, when the transcript has actually painted:
 *     historyDone();
 *
 * Gates are NAMED so a stuck reveal is diagnosable (`pendingGates()` →
 * `["history"]`) rather than an unexplained hang.
 *
 * ## Mount gating vs reveal gating — do not blur these
 *
 * `block.tsx:380-398` records a P0 DEADLOCK from exactly that mistake: folding a
 * gate into `ready()` meant `BlockFull` never mounted, but mounting was the only
 * thing that triggered the backfill the gate waited on. Gate and producer
 * deadlocked.
 *
 * Every gate registered here is a REVEAL gate. The content is already mounted and
 * running underneath the cover, so a gate can safely wait on anything the mounted
 * content itself produces. Nothing in this module may ever be used to decide
 * whether a component mounts.
 *
 * ## Never hangs
 *
 * A single authority is also a single point of hang: one gate that never reports
 * would hide the pane forever. `revealTimeoutMs` bounds that — on expiry the pane
 * reveals anyway and logs which gates were outstanding, degrading to today's
 * behaviour rather than an indefinite cover.
 */

import { createSignal, onCleanup } from "solid-js";

export type PaneReadinessPhase = "assembling" | "revealing" | "live";

export interface PaneReadiness {
    /** Current phase. `assembling` → `revealing` → `live`. */
    phase: () => PaneReadinessPhase;
    /**
     * Declare a reveal-blocking dependency. Returns its completion callback,
     * which is idempotent — calling it twice does not release a second gate.
     */
    gate: (name: string) => () => void;
    /** True until `live`. For chrome that wants to suppress transient affordances. */
    isLoading: () => boolean;
    /** Names of gates still outstanding — for diagnostics, not control flow. */
    pendingGates: () => string[];
    /** Called by the cover when its fade finishes, moving `revealing` → `live`. */
    revealComplete: () => void;
}

export interface PaneReadinessOptions {
    /**
     * How long `assembling` may last before revealing anyway. Matches the spirit of
     * the 2s bound `waitForMuxObjectSettled` already uses for a stuck MOS fetch.
     */
    revealTimeoutMs?: number;
    /** Identifies the pane in the timeout warning. */
    label?: string;
    /** Seam for tests. */
    onTimeout?: (pending: string[]) => void;
}

const DEFAULT_REVEAL_TIMEOUT_MS = 8000;

/**
 * Creates a readiness controller. Call inside a reactive owner — the timeout is
 * cleared on cleanup so a disposed pane cannot fire it.
 */
export function createPaneReadiness(opts: PaneReadinessOptions = {}): PaneReadiness {
    const revealTimeoutMs = opts.revealTimeoutMs ?? DEFAULT_REVEAL_TIMEOUT_MS;
    const [phase, setPhase] = createSignal<PaneReadinessPhase>("assembling");
    const [pending, setPending] = createSignal<string[]>([]);

    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    // Gates registered so far. A pane that registers none reveals immediately, so
    // non-agent panes are unaffected by this module existing.
    let everRegistered = false;

    const clearTimer = () => {
        if (timeoutId != null) {
            clearTimeout(timeoutId);
            timeoutId = null;
        }
    };

    const toRevealing = () => {
        if (phase() !== "assembling") return;
        clearTimer();
        setPhase("revealing");
    };

    const armTimeout = () => {
        if (timeoutId != null) return;
        timeoutId = setTimeout(() => {
            timeoutId = null;
            const stuck = pending();
            if (phase() === "assembling") {
                // Loud on purpose: a pane that silently recovers still hides a bug.
                console.error(
                    `[pane-readiness] ${opts.label ?? "pane"}: revealing after ${revealTimeoutMs}ms with ` +
                        `gate(s) still pending: ${stuck.join(", ") || "(none)"}. ` +
                        `A gate was registered and never completed.`
                );
                opts.onTimeout?.(stuck);
                toRevealing();
            }
        }, revealTimeoutMs);
    };

    const gate = (name: string): (() => void) => {
        if (phase() !== "assembling") {
            // Registering after reveal is a no-op rather than an error: a late
            // dependency must not re-hide a pane the user is already looking at.
            return () => {};
        }
        everRegistered = true;
        setPending((p) => [...p, name]);
        armTimeout();

        let released = false;
        return () => {
            if (released) return;
            released = true;
            setPending((p) => {
                const idx = p.indexOf(name);
                if (idx === -1) return p;
                const next = [...p];
                next.splice(idx, 1);
                return next;
            });
            if (pending().length === 0) toRevealing();
        };
    };

    const revealComplete = () => {
        if (phase() === "live") return;
        clearTimer();
        setPhase("live");
    };

    // A pane that never registers a gate has nothing to wait for. Defer by a
    // microtask so synchronous registration during setup still counts.
    queueMicrotask(() => {
        if (!everRegistered && phase() === "assembling") toRevealing();
    });

    onCleanup(clearTimer);

    return {
        phase,
        gate,
        isLoading: () => phase() !== "live",
        pendingGates: pending,
        revealComplete,
    };
}
