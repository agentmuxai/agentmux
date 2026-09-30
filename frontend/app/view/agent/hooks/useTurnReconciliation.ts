// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 10).

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { BlockService } from "@/app/store/services";
import { createEffect, on, type Accessor } from "solid-js";

/**
 * Focus/visibility-triggered re-poll — the mount-time GetControllerStatus
 * (onControllerStatus in agent-view.tsx) is one-shot, and the live
 * useControllerStatusEvents subscription only self-heals a missed turn-end if
 * a LATER live event arrives. If the single turn-end push is missed
 * (backgrounded window, a MPS reconnect gap, a pane remount that doesn't
 * re-trigger the MPS persisted-event replay — see
 * REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md §3/§4 item 5)
 * nothing else corrects it until the *next* turn starts. Re-poll on every
 * background→foreground transition to drive turnJustEnded edge-tracking and
 * the deferred controller-refresh recovery, independent of event-bus replay
 * semantics. Skips the initial `true` at mount (already covered by the
 * one-shot) via `{ defer: true }`.
 *
 * Deliberately does NOT call reconcileTurnActive from this snapshot (removed
 * per direct user request — "Working" state must not depend on window focus
 * at all). `turn_active` isn't a clean boolean: it reads transiently false
 * during the gap between one CLI round's session_end and the next round's
 * start (the same phenomenon StreamFlushObserved's Done->Streaming
 * re-promotion exists to paper over on a different path), and this poll fires
 * on the single most common user action — clicking/refocusing a pane to check
 * on it — making that race far more visible than it needs to be. The live
 * useControllerStatusEvents subscription still reconciles TurnPhase from the
 * backend's periodic status heartbeat (persistent.rs's spawn_status_heartbeat,
 * every 20s while a turn is active) independent of focus, so the original
 * stuck-Working-forever gap this mechanism was built for is still bounded —
 * just by that heartbeat's cadence instead of an instant refocus.
 */
export function useFocusRepoll(opts: {
    blockId: string;
    windowFocused: Accessor<boolean>;
    trackTurnJustEnded: (active: boolean) => void;
    flushPendingControllerRefresh: () => unknown;
    getControllerStatus?: (blockId: string) => Promise<{ turn_active?: boolean } | null | undefined>;
}): void {
    const getControllerStatus = opts.getControllerStatus ?? ((id: string) => BlockService.GetControllerStatus(id));
    createEffect(
        on(
            opts.windowFocused,
            (focused) => {
                if (!focused) return;
                void getControllerStatus(opts.blockId)
                    .then((rts) => {
                        if (!rts) return;
                        const active = !!rts.turn_active;
                        // Mirror the live useControllerStatusEvents handler —
                        // reagent P2: a turn-end detected ONLY via this focus poll
                        // (the missed-live-push case this mechanism exists for)
                        // must still bump turnJustEndedAtom, or
                        // useAgentActivitySummary/useNextPromptSuggestion silently
                        // never fire for that turn's completion.
                        opts.trackTurnJustEnded(active);
                        // Independent of the turnJustEnded edge above: this RPC
                        // response is itself a fresh, authoritative confirmation of
                        // idleness whenever active is false — attempt the deferred
                        // refresh unconditionally on that, not only when
                        // trackTurnJustEnded's edge detector fires. didTurnJustEnd
                        // requires prev===true (a CONFIRMED active state to
                        // transition FROM); a pane whose backend state was never
                        // confirmed either way before this poll (wasTurnActive
                        // undefined — e.g. the live confirming controllerstatus
                        // push was itself missed, the exact gap this poll exists to
                        // self-heal) computes turnJustEnded=false here even though
                        // this is the FIRST time idleness has been confirmed. The
                        // reactive turnPhase effect (useHeldMessageDelivery) can't
                        // rescue this either: ReconcileTurnActive no-ops (same state
                        // reference) once local turnPhase already reads idle/Done,
                        // so it never re-fires off this same confirmation. Without
                        // this call, a /login deferred mid-turn — where the turn
                        // then ends via session_end while the live idle
                        // controllerstatus push is lost — would leave the refresh
                        // (and any held messages) stuck until the user happens to
                        // send another message. codex P1 on PR #2338
                        // (twenty-eighth re-review).
                        if (!active) {
                            void opts.flushPendingControllerRefresh();
                        }
                    })
                    .catch(() => {
                        // Best-effort — the live subscription and next mount remain
                        // as fallbacks; nothing user-visible to report on failure.
                    });
            },
            { defer: true }
        )
    );
}

/**
 * Deliver queued-while-busy ("send now") messages at the next tool-call
 * boundary — the agent finishes its current step and then picks them up (the
 * CLI consumes a stdin message at its next inference, after the in-flight
 * tool's result). Falls back to turn end (Idle/Done) so a tool-less turn still
 * delivers. Holding until here is what lets ArrowUp recall an un-sent message
 * first.
 */
export function useHeldMessageDelivery(opts: {
    paneModel: Pick<AgentPaneModel, "state">;
    hasHeldMessages: () => boolean;
    flushHeldMessages: () => unknown;
    flushPendingControllerRefresh: () => unknown;
}): void {
    const { paneModel } = opts;
    let prevTool: string | null = null;
    createEffect(() => {
        const tool = paneModel.state.currentTool;
        const phaseKind = paneModel.state.turnPhase.kind;
        const newToolCall = tool !== null && tool !== prevTool;
        prevTool = tool;
        const turnIdle = phaseKind === "Idle" || phaseKind === "Done";
        if ((newToolCall || turnIdle) && opts.hasHeldMessages()) {
            void opts.flushHeldMessages();
        }
        // Independent of the above: run any controller refresh /login
        // deferred because a turn was active when it succeeded, the moment
        // this pane's OWN turnPhase reflects idle — regardless of whether
        // there are any held messages to otherwise trigger it. Deliberately
        // reacts to turnPhase directly rather than relying solely on
        // trackTurnJustEnded's live-controllerstatus-event edge detector:
        // (1) a turn also ends via the independent session_end -> TurnEnd
        // stream path (useTurnLifecycle.ts's finalizeTurn), which is not
        // synchronized with the controllerstatus event stream reagent P1
        // found flushHeldMessages/trackTurnJustEnded alone don't cover; (2)
        // a pane that mounts onto an ALREADY-active turn never initializes
        // trackTurnJustEnded's wasTurnActive (deliberately, to avoid a
        // false busy->idle edge on the very first live event — see its own
        // doc comment), so if /login succeeds during that pre-existing turn
        // and it ends before any OTHER live event arrives,
        // didTurnJustEnd(undefined, false) never fires and — with no held
        // messages either — nothing would ever run the deferred refresh at
        // all. Codex P1 on PR #2338 (seventeenth re-review, both points).
        // No-ops when nothing is pending.
        if (turnIdle) {
            void opts.flushPendingControllerRefresh();
        }
    });
}
