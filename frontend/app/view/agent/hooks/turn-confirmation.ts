// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 10).

import { createSignal, type Accessor } from "solid-js";
import { didTurnJustEnd } from "./useControllerStatusEvents";

export interface TurnConfirmation {
    /** Bumped once per backend-confirmed turn end (true -> false edge, live events only). */
    turnJustEndedAtom: Accessor<number>;
    /** Dispatch ReconcileTurnActive; does not move the edge detector. */
    reconcileTurnActive: (active: boolean) => void;
    /** Feed a LIVE controllerstatus reading into the edge detector. */
    trackTurnJustEnded: (active: boolean) => void;
    /** The last confirmed live reading was "active". */
    isBackendTurnActive: () => boolean;
    /** The last confirmed live reading was "idle" (never true before any reading). */
    isBackendTurnConfirmedIdle: () => boolean;
}

/**
 * The one owner of the backend-confirmed turn state that the view writes and
 * useAgentCommands reads (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.3).
 */
export function createTurnConfirmation(opts: {
    /** Dispatch ReconcileTurnActive to the pane reducer. */
    reconcile: (active: boolean) => void;
    /** A turn just ended. Runs after the state has updated. */
    onTurnEnded: () => void;
}): TurnConfirmation {
    // Bumped exactly once per genuine, backend-confirmed turn completion —
    // the `turn_active: true -> false` edge, fed ONLY by live controllerstatus
    // events (see trackTurnJustEnded below, and NOT reconcileTurnActive — the
    // mount-time one-shot deliberately does not participate; reagent P1 on
    // PR #2241). This is the trigger useAgentActivitySummary/
    // useNextPromptSuggestion use instead of TurnPhase.kind === "Done" (which
    // over-triggers — see
    // docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md).
    // `wasTurnActive` is plain (non-reactive) — it only exists to detect the
    // edge, not to be read anywhere.
    let wasTurnActive: boolean | undefined;
    const [turnJustEndedAtom, setTurnJustEndedAtom] = createSignal(0);

    // Dispatches ReconcileTurnActive to the pane reducer so TurnPhase follows
    // the backend's live turn state — used by BOTH the mount-time one-shot
    // (useAgentControllerStatus's Phase 3 GetControllerStatus) and every live
    // controllerstatus event. Does NOT touch turnJustEndedAtom — see
    // trackTurnJustEnded for why that's kept separate.
    function reconcileTurnActive(active: boolean): void {
        opts.reconcile(active);
    }

    // Feeds the turnJustEndedAtom edge-detector. Deliberately called ONLY
    // from the live useControllerStatusEvents subscription (up from onMount,
    // always current), never from the mount-time GetControllerStatus
    // one-shot. That one-shot can resolve up to ~300s late — after Phase 1/2's
    // auth wait — by which point the live subscription may have already
    // tracked a real turn starting AND ending. Letting the stale snapshot
    // also drive wasTurnActive could clobber the correct live-tracked state
    // back to a value that no longer reflects reality, making the next live
    // event compute a spurious edge and re-fire the Haiku RPC for a turn that
    // isn't actually ending — reintroducing the over-trigger bug this fix
    // closes (reagent P1 on PR #2241).
    function trackTurnJustEnded(active: boolean): void {
        const turnJustEnded = didTurnJustEnd(wasTurnActive, active);
        // Update BEFORE calling flushPendingControllerRefresh below, not
        // after: that call synchronously checks isBackendTurnConfirmedIdle()
        // (backed by this same wasTurnActive) at call time, before any
        // await — the OLD ordering left it reading the STALE (pre-update)
        // value on exactly the genuine turn-end edge this call exists to
        // react to, so the deferred refresh's own safety gate saw the
        // turn as still "active" and refused to run — stranding it
        // forever on this trigger (the reactive turnIdle effect could
        // still rescue it asynchronously, but only if it happened to fire
        // separately). Codex P1 on PR #2338 (twenty-first re-review).
        wasTurnActive = active;
        if (turnJustEnded) {
            setTurnJustEndedAtom((n) => n + 1);
            // Run any controller refresh /login deferred because this exact
            // turn was still active when it succeeded — see
            // SlashCommandContext.deferControllerRefreshUntilIdle's doc
            // comment. No-ops if nothing is pending.
            opts.onTurnEnded();
        }
    }

    return {
        turnJustEndedAtom,
        reconcileTurnActive,
        trackTurnJustEnded,
        isBackendTurnActive: () => wasTurnActive === true,
        isBackendTurnConfirmedIdle: () => wasTurnActive === false,
    };
}
