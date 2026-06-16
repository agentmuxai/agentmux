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
import * as WOS from "@/app/store/wos";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseControllerStatusEventsOptions {
    blockId: string;
    log: LogFn;
}

export function useControllerStatusEvents(opts: UseControllerStatusEventsOptions): void {
    onMount(() => {
        const unsubStatus = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const status = (event as any)?.data?.shellprocstatus;
                if (status === "running") {
                    opts.log("subprocess", "spawned, waiting for response...");
                } else if (status === "done") {
                    const exitCode = (event as any)?.data?.shellprocexitcode;
                    if (exitCode != null && exitCode !== 0) {
                        opts.log("subprocess", `exited with code ${exitCode}`, "error");
                    } else {
                        opts.log("subprocess", "turn complete");
                    }
                }
            },
        });

        // Rich failure cause emitted alongside a non-zero exit
        // (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2): surfaces the real reason —
        // auth, rate-limit, OOM, context, etc. — plus the stderr tail, instead of
        // just "exited with code N".
        const unsubFailure = waveEventSubscribe({
            eventType: "agentfailure",
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                let msg = f.title || "Agent run failed";
                if (f.detail) msg += ` — ${f.detail}`;
                if (f.exitCode != null) msg += ` [exit ${f.exitCode}]`;
                else if (f.signal != null) msg += ` [signal ${f.signal}]`;
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
