// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useControllerStatusEvents — subscribes to the `controllerstatus`
 * wave event (scoped to one block) and translates shellprocstatus /
 * shellprocexitcode into log lines.
 *
 * Step 12 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 */

import { onCleanup, onMount } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import type { LogFn } from "./useAgentControllerStatus";

/**
 * Derive the reconciled `turn_active` from a raw controllerstatus event's
 * `data`, or `null` when the event carries no turn signal (non-agent panes).
 *
 * Encodes the wire contract: `BlockControllerRuntimeStatus.is_agent_pane` and
 * `.turn_active` are both `#[serde(skip_serializing_if = "is_false")]`
 * (agentmux-srv/src/backend/blockcontroller/mod.rs), so a `false` value is
 * OMITTED from the JSON rather than sent as `false`. A turn-END event for an
 * agent pane therefore looks like `{ is_agent_pane: true }` with `turn_active`
 * absent — which must read as `false`, not "no signal". Only a present
 * `is_agent_pane: true` distinguishes an agent pane (idle or busy) from a
 * shell/PTY pane (`{}`), so it is the gate; the turn state is then
 * `turn_active === true` (absent → false).
 */
export function deriveTurnActive(data: unknown): boolean | null {
    if (!data || typeof data !== "object") return null;
    const d = data as { is_agent_pane?: unknown; turn_active?: unknown };
    if (d.is_agent_pane !== true) return null;
    return d.turn_active === true;
}

/**
 * True exactly when a turn genuinely just ended: the last-known state was
 * confirmed `true` (busy) and this event reports `false` (idle). `prev` is
 * `undefined` before any turn_active-bearing event has ever been seen for
 * this pane's current mount — a fresh pane opened onto an already-idle agent
 * reports `false` with `prev === undefined`, which must NOT count as a
 * just-ended turn (there's nothing new to summarize).
 *
 * This is the trigger for `useAgentActivitySummary`/`useNextPromptSuggestion`
 * (their `turnJustEndedAtom`, see agent-view.tsx's `reconcileTurnActive`) —
 * deliberately independent of the frontend's own `TurnPhase.kind === "Done"`,
 * which fires on several non-terminal transitions (a premature per-round
 * `session_end`, bounded-timeout force-transitions) that don't correspond to
 * a real backend turn_active flip. See
 * docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md.
 */
export function didTurnJustEnd(prev: boolean | undefined, next: boolean): boolean {
    return prev === true && next === false;
}

export interface UseControllerStatusEventsOptions {
    blockId: string;
    log: LogFn;
    /**
     * Live turn-active reconciliation. Called on every controllerstatus event
     * that carries a boolean `turn_active` (agent panes only), so the pane's
     * TurnPhase can follow the backend's authoritative turn state in BOTH
     * directions while mounted — not just the one-shot GetControllerStatus at
     * mount. This is what lets a stuck `Streaming` phase demote to `Idle` when
     * the backend reports the turn ended but the frontend missed the terminal
     * `session_end` (Agent1/Agent2 incidents). Consumer dispatches
     * `ReconcileTurnActive`. See docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md.
     */
    onTurnActive?: (active: boolean) => void;
    /**
     * Fires ONLY when a controllerstatus event proves a turn is genuinely
     * running right now (`active === true`) — never for an idle/heartbeat
     * event. An idle event merely means an agent-pane-type controller
     * exists; it says nothing about whether ITS credential is currently
     * valid. A persistent controller left alive from before a just-failed
     * recovery attempt still emits these, so a caller using "any
     * controllerstatus event arrived" as proof of health (as this hook's
     * `onTurnActive` used to be used for) would have a stray idle event
     * silently clear that recovery's own `canRetry=true`, letting the very
     * next message bypass the fast-fail guard and reach the known-bad
     * process. Codex P1 on PR #2338 (eighth re-review).
     */
    onActiveTurnConfirmed?: () => void;
}

export function useControllerStatusEvents(opts: UseControllerStatusEventsOptions): void {
    onMount(() => {
        const unsubStatus = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const data = (event as any)?.data;
                const status = data?.shellprocstatus;
                if (status === "running") {
                    opts.log("subprocess", "spawned, waiting for response...");
                } else if (status === "done") {
                    const exitCode = data?.shellprocexitcode;
                    if (exitCode != null && exitCode !== 0) {
                        opts.log("subprocess", `exited with code ${exitCode}`, "error");
                    } else {
                        opts.log("subprocess", "turn complete");
                    }
                }
                // Reconcile turn_active for agent panes only — see
                // deriveTurnActive for the omitted-false wire contract that
                // makes a turn-END event's `turn_active` ABSENT (not `false`).
                const active = deriveTurnActive(data);
                if (opts.onTurnActive && active !== null) {
                    opts.onTurnActive(active);
                }
                if (active === true) {
                    opts.onActiveTurnConfirmed?.();
                }
            },
        });

        // Rich failure cause emitted alongside a non-zero exit
        // (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2): surfaces the real reason —
        // auth, rate-limit, OOM, context, etc. — plus the stderr tail, instead of
        // just "exited with code N".
        const unsubFailure = waveEventSubscribe({
            eventType: WpsEvent.AgentFailure,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                let msg = f.title || "Agent run failed";
                if (f.detail) msg += ` — ${f.detail}`;
                // Prefer signal when present (matches AgentFailure::explain(); a
                // signal kill stores exitCode as -1, so "[exit -1]" would mislead).
                if (f.signal != null) msg += ` [signal ${f.signal}]`;
                else if (f.exitCode != null) msg += ` [exit ${f.exitCode}]`;
                if (f.retryable) msg += " (retryable)";
                opts.log("subprocess", msg, "error");
                if (f.stderrTail) {
                    opts.log("subprocess", `claude stderr (tail):\n${f.stderrTail}`, "error");
                }
            },
        });

        onCleanup(() => {
            unsubStatus();
            unsubFailure();
        });
    });
}
