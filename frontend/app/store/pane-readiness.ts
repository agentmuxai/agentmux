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
 * ## A stuck gate is reported, NOT force-revealed
 *
 * A single authority is also a single point of hang: one gate that never reports
 * would hide the pane forever. The tempting mitigation — reveal anyway after N
 * seconds — is wrong here, and an earlier draft of this module got it wrong:
 *
 *   - The behaviour it replaces waits INDEFINITELY for its two conditions. A
 *     forced reveal is therefore a behaviour change, not a safety net.
 *   - The slow case is the legitimate one. A persisted-session agent pane replays
 *     a large transcript, settles auth, and backfills subagents; that is exactly
 *     the load most likely to exceed any timeout, and exactly the pane this cover
 *     exists to hide while it assembles. Revealing it half-built is the flicker
 *     this consolidation is meant to eliminate.
 *
 * So by default the cover stays until every gate reports, and `warnAfterMs` only
 * LOGS which gates are outstanding — full diagnosability, zero behaviour change.
 * A caller that genuinely wants a bound opts in with `revealTimeoutMs`; nothing
 * does today.
 *
 * The two are INDEPENDENT deadlines on independent timers. Collapsing them into
 * one timer at `Math.min(...)` makes a caller's explicit hard bound silently
 * shrink to the warn interval — see the comment on `armTimers`.
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
     * Log (do NOT reveal) if gates are still outstanding after this long. Pure
     * diagnostics — the cover stays up. Defaults to 8s.
     */
    warnAfterMs?: number;
    /**
     * Opt-in hard bound: force the reveal if gates are still outstanding after
     * this long. **Off by default, deliberately** — see the module comment. Only
     * set this for a surface where showing partial content genuinely beats waiting.
     */
    revealTimeoutMs?: number;
    /** Identifies the pane in the warning. */
    label?: string;
    /**
     * Seam for tests. Fires with the outstanding gates each time a deadline
     * elapses while still assembling — so a caller that sets both options and
     * misses both deadlines sees it twice, once per deadline.
     */
    onTimeout?: (pending: string[]) => void;
}

const DEFAULT_WARN_AFTER_MS = 8000;

/**
 * Creates a readiness controller. Call inside a reactive owner — both deadline
 * timers are cleared on cleanup so a disposed pane cannot fire them.
 */
export function createPaneReadiness(opts: PaneReadinessOptions = {}): PaneReadiness {
    const warnAfterMs = opts.warnAfterMs ?? DEFAULT_WARN_AFTER_MS;
    const revealTimeoutMs = opts.revealTimeoutMs;
    const [phase, setPhase] = createSignal<PaneReadinessPhase>("assembling");
    const [pending, setPending] = createSignal<string[]>([]);

    // Two INDEPENDENT deadlines, deliberately two timers. An earlier revision
    // armed a single timer at `Math.min(warnAfterMs, revealTimeoutMs)` and
    // force-revealed whenever it fired, so `revealTimeoutMs: 20000` against the
    // default 8s warn revealed at 8s — silently breaking the hard bound the
    // caller asked for. The deadlines mean different things; they don't collapse.
    // (reagent P1 on #3462, round 2.)
    let warnTimerId: ReturnType<typeof setTimeout> | null = null;
    let revealTimerId: ReturnType<typeof setTimeout> | null = null;
    // Each deadline is one-shot. Without this, a gate registered after a
    // deadline already elapsed would re-arm it (the id is back to null) and
    // warn again about the same stuck pane.
    let warnFired = false;
    let revealFired = false;
    // Gates registered so far. A pane that registers none reveals immediately, so
    // non-agent panes are unaffected by this module existing.
    let everRegistered = false;

    const clearTimers = () => {
        if (warnTimerId != null) {
            clearTimeout(warnTimerId);
            warnTimerId = null;
        }
        if (revealTimerId != null) {
            clearTimeout(revealTimerId);
            revealTimerId = null;
        }
    };

    const toRevealing = () => {
        if (phase() !== "assembling") return;
        clearTimers();
        setPhase("revealing");
    };

    const armTimers = () => {
        if (warnTimerId == null && !warnFired) {
            warnTimerId = setTimeout(() => {
                warnTimerId = null;
                warnFired = true;
                if (phase() !== "assembling") return;
                const stuck = pending();
                // Loud on purpose: a pane stuck behind a cover is a bug worth
                // seeing, even though we do NOT hide it by revealing half-built
                // content.
                console.error(
                    `[pane-readiness] ${opts.label ?? "pane"}: still assembling after ${warnAfterMs}ms; ` +
                        `gate(s) pending: ${stuck.join(", ") || "(none)"}. ` +
                        `Still waiting — the cover stays up until every gate reports` +
                        (revealTimeoutMs != null ? ` or revealTimeoutMs=${revealTimeoutMs}ms elapses.` : `.`)
                );
                opts.onTimeout?.(stuck);
            }, warnAfterMs);
        }
        // Only an explicit opt-in bound reveals early. The default leaves this
        // timer unarmed entirely, which is what the pre-consolidation code did.
        if (revealTimeoutMs != null && revealTimerId == null && !revealFired) {
            revealTimerId = setTimeout(() => {
                revealTimerId = null;
                revealFired = true;
                if (phase() !== "assembling") return;
                const stuck = pending();
                console.error(
                    `[pane-readiness] ${opts.label ?? "pane"}: Forcing reveal after ` +
                        `revealTimeoutMs=${revealTimeoutMs}ms; gate(s) never reported: ` +
                        `${stuck.join(", ") || "(none)"}.`
                );
                opts.onTimeout?.(stuck);
                toRevealing();
            }, revealTimeoutMs);
        }
    };

    const gate = (name: string): (() => void) => {
        if (phase() !== "assembling") {
            // Registering after reveal is a no-op rather than an error: a late
            // dependency must not re-hide a pane the user is already looking at.
            return () => {};
        }
        everRegistered = true;
        setPending((p) => [...p, name]);
        armTimers();

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
        clearTimers();
        setPhase("live");
    };

    // A pane that never registers a gate has nothing to wait for. Defer by a
    // microtask so synchronous registration during setup still counts.
    queueMicrotask(() => {
        if (!everRegistered && phase() === "assembling") toRevealing();
    });

    onCleanup(clearTimers);

    return {
        phase,
        gate,
        isLoading: () => phase() !== "live",
        pendingGates: pending,
        revealComplete,
    };
}
