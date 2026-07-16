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
                // `turn_active` is only meaningful for agent panes (shell/PTY
                // controllers leave it absent); a non-boolean means "no signal",
                // not "idle", so only forward a real boolean.
                if (opts.onTurnActive && data?.is_agent_pane && typeof data.turn_active === "boolean") {
                    opts.onTurnActive(data.turn_active);
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
