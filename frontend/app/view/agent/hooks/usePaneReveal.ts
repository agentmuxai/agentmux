// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 8).

import { createPaneReadiness, type PaneReadiness } from "@/app/store/pane-readiness";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type Accessor } from "solid-js";
import { agentOpenRevealed, beginAgentOpenOnMount, finishAgentOpen, markAgentOpen, noteAgentOpen } from "../open-trace";

/** The launch-flow state the reveal waits on (useAgentControllerStatus). */
export interface RevealLaunchStatus {
    launchPhase: Accessor<{ kind: string } | null>;
    authStatus: Accessor<string>;
}

/** Timing primitives, injectable for tests; the real ones by default. */
export interface PaneRevealDeps {
    scheduleOnSettle: (onSettled: () => void) => () => void;
    frame: (cb: () => void) => number;
    cancelFrame: (id: number) => void;
}

const defaultDeps: PaneRevealDeps = {
    scheduleOnSettle: (cb) => scheduleOnSettle(cb),
    frame: (cb) => requestAnimationFrame(cb),
    cancelFrame: (id) => cancelAnimationFrame(id),
};

export interface PaneReveal {
    readiness: PaneReadiness;
    /** History has been dispatched: wait for it to actually paint, then release the history gate. */
    startPaintWait: () => void;
    /** Call once `status` exists: holds the reveal for the launch flow, then fades. */
    connectLaunchStatus: (status: RevealLaunchStatus) => void;
}

export function usePaneReveal(opts: {
    blockId: string;
    agentName: Accessor<string>;
    deps?: Partial<PaneRevealDeps>;
}): PaneReveal {
    const deps: PaneRevealDeps = { ...defaultDeps, ...opts.deps };
    // Brain-spinner loading overlay (see
    // docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md):
    // shown from mount, cross-fades out once real content has actually
    // painted, so a content-heavy pane never sits blank while it replays.
    //
    // `onHistoryReady` fires right after the NDJSON parse/dispatch — BEFORE
    // the resulting DOM's layout/paint, which is the dominant cost for heavy
    // sessions (500-600ms, see SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md).
    // Starting the fade there (or after any flat delay) would make the
    // overlay disappear before painting finishes for exactly the
    // content-heavy case this exists to cover — reproducing the blank
    // window instead of fixing it. `scheduleOnSettle` (same Long-Task-quiet
    // detector `tab-reveal.ts` uses for the analogous tab-switch case) waits
    // for the main thread to actually go quiet post-dispatch before the
    // fade starts. `showLoadingOverlay` then unmounts the overlay entirely
    // once the fade transition has had time to finish, instead of leaving
    // an invisible-but-present pointer-events:none div forever.
    // One readiness authority for this pane — see
    // docs/specs/SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md. Phase 1 changes no
    // behaviour: the same two conditions gate the reveal, the fade still runs for
    // 220ms, and the overlay still unmounts after it. What changes is that "may the
    // pane appear" is now ONE stated decision instead of several components each
    // deciding independently — a prerequisite for collapsing the four overlapping
    // loading indicators (two were measured on screen at once) in later phases.
    //
    // The phase maps onto exactly what the two old booleans encoded:
    //   assembling → covered, not yet fading   (was: !historyLoaded && showOverlay)
    //   revealing  → covered, fading            (was:  historyLoaded && showOverlay)
    //   live       → unmounted                  (was: !showOverlay)
    beginAgentOpenOnMount(opts.blockId, opts.agentName());
    onCleanup(() => finishAgentOpen(opts.blockId, "closed"));
    const windowTabDisplayed = useWindowTabDisplayed();
    const readiness = createPaneReadiness({
        label: `block:${opts.blockId}`,
        holdFor: opts.blockId,
        hidden: () => !windowTabDisplayed(),
    });
    const releaseHistoryGate = readiness.gate("history");
    const releaseAuthGate = readiness.gate("auth");
    // Separate from `historyLoaded` below: this only means "the transcript
    // has actually painted" — the effect after `status` is defined (further
    // down) decides whether that's enough to start the fade, or whether the
    // auth-panel pop-in flicker fix also needs to hold the overlay a bit
    // longer.
    const [historyPainted, setHistoryPainted] = createSignal(false);
    let cancelSettleWait: (() => void) | undefined;
    let loadingOverlayFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    // Two extra rAFs between "settle detected" and actually starting the
    // fade — see the doc comment on scheduleOnSettle's call site below for
    // why: Long-Task quiet alone can be reached before the browser has
    // actually PAINTED this pane's content (live-reported flicker,
    // 2026-08-11). Tracked so a pane close mid-transition doesn't write to
    // disposed signals.
    let settlePaintRaf1: number | undefined;
    let settlePaintRaf2: number | undefined;
    onCleanup(() => {
        cancelSettleWait?.();
        clearTimeout(loadingOverlayFadeTimeout);
        if (settlePaintRaf1 !== undefined) deps.cancelFrame(settlePaintRaf1);
        if (settlePaintRaf2 !== undefined) deps.cancelFrame(settlePaintRaf2);
    });

    const startPaintWait = () => {
        cancelSettleWait = deps.scheduleOnSettle(() => {
            // `scheduleOnSettle` only watches for Long-Task quiet
            // (no synchronous block >50ms) — but this pane's actual
            // reveal work (AgentDocumentVirtualList's measure
            // ResizeObserver, scroll-pin/anchor-restore effects) is
            // spread across several async/RAF-scheduled steps that
            // never register as one long task each. "No long tasks
            // observed" can therefore be reached before the browser has
            // actually PAINTED the resulting rows — starting the fade
            // there let the spinner finish disappearing while the pane
            // was still genuinely blank underneath, then the real
            // content popped in abruptly once that async chain finally
            // caught up (live-reported flicker, 2026-08-11, repro:
            // switch tabs, screenshot-burst the transition — spinner
            // fully faded by ~400ms, content not visible until ~500ms).
            // A double requestAnimationFrame is the standard "wait for
            // an actual paint to have happened" technique: the second
            // callback is guaranteed to run only after whatever was
            // queued as of the first one's frame has been painted.
            settlePaintRaf1 = deps.frame(() => {
                settlePaintRaf2 = deps.frame(() => {
                    markAgentOpen(opts.blockId, "painted");
                    setHistoryPainted(true);
                });
            });
        });
    };

    const connectLaunchStatus = (status: RevealLaunchStatus) => {
        // Brain-spinner loading overlay, part 2 (part 1 is the historyPainted/
        // scheduleOnSettle chain above `status`'s own definition — this has to
        // live down here since it reads `status.launchPhase()`). A fresh
        // agent's mount-time launch-flow.ts runs Phase 1 (resolving-cli) then
        // Phase 2 (checking-auth) before AgentAuthPanel's authUrl/authNotice can
        // ever become non-null; when they do, that panel pops into normal flex
        // flow between the scroll region and the composer strip/AgentFooter,
        // pushing both down with no warning — often well after historyPainted
        // already flipped true for a brand-new, empty-history agent (live-
        // reported flicker, 2026-08-17: "bottom paints first, then gets pushed
        // down"). Hold the fade until the launch flow has moved past the two
        // phases that can still cause that pop-in, so it happens hidden behind
        // the mask instead — same principle as the historyPainted rAF-pair fix
        // above, just gating on a different async source. Bounded by a 3s
        // safety timeout so a launch path that never calls setLaunchPhase (a
        // future code path, a test double) can't leave the pane stuck behind
        // the spinner forever — worse than the flicker this exists to fix.
        const [authPhaseTimedOut, setAuthPhaseTimedOut] = createSignal(false);
        let authPhaseSafetyTimeout: ReturnType<typeof setTimeout> | undefined;
        onMount(() => {
            authPhaseSafetyTimeout = setTimeout(() => setAuthPhaseTimedOut(true), 3000);
        });
        onCleanup(() => clearTimeout(authPhaseSafetyTimeout));
        const authPhaseSettled = createMemo(() => {
            if (authPhaseTimedOut()) return true;
            const phase = status.launchPhase();
            return phase !== null && phase.kind !== "resolving-cli" && phase.kind !== "checking-auth";
        });
        // Report each dependency to the readiness controller as it completes, rather
        // than re-deriving "are we done yet" from a conjunction. Same two conditions,
        // same resulting moment — but now each one STATES that it is finished, so a
        // stuck reveal names the gate (`readiness.pendingGates()`) instead of being an
        // unexplained hang. Both releases are idempotent, so re-running this effect on
        // an unrelated signal change is harmless.
        createEffect(() => {
            if (historyPainted()) releaseHistoryGate();
        });
        createEffect(() => {
            if (authPhaseSettled()) releaseAuthGate();
        });
        // `revealing` → `live`: hold the overlay mounted for the fade's own duration
        // (matching PaneLoadingCover.scss's transition) before unmounting, so it fades
        // as one visual unit with the spinner instead of vanishing mid-transition.
        createEffect(() => {
            if (readiness.phase() === "revealing") {
                // Where the open landed: `first-login`/`auth-expired` means the
                // pane now waits on the user, which the open's time excludes.
                noteAgentOpen(opts.blockId, {
                    auth: untrack(() => status.launchPhase()?.kind ?? status.authStatus()),
                });
                agentOpenRevealed(opts.blockId);
                loadingOverlayFadeTimeout = setTimeout(() => readiness.revealComplete(), 220);
            }
        });
    };

    return { readiness, startPaintWait, connectLaunchStatus };
}
