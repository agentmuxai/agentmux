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
        const unsub = waveEventSubscribe({
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
        onCleanup(() => unsub());
    });
}
