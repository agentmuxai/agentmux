// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host's process-message lane for the one command the page sends every frame.
 *
 * Host commands go over HTTP (`ipc.ts`). The browser panes' rect batch (`browser_panes_set_rects`) goes
 * out once per window per frame while a pane border or the window is dragged, and HTTP cost it 7 to 9 ms
 * and one of the page's six connections every time, so a pane's body trailed the chrome around it. The
 * host's renderer puts `window.agentmuxQuery` on an app window's main frame (crates/cef/src/ipc_query.rs):
 * a call is one process message to the host and one back.
 *
 * `invokeFastCommand` uses the lane when the window has it, and HTTP when it doesn't or when the host
 * refuses the query (an older host with no handler for it, a token the host didn't accept): the command
 * still runs, as before, only slower. A host with no handler is remembered, so the page doesn't ask twice.
 */

import { recordIpcRoundtrip } from "@/perf";
import { invokeCommand } from "./ipc";

/** The router's own failure code: no handler took the query. */
const NO_HANDLER = -1;

class QueryFailure extends Error {
    constructor(
        readonly code: number,
        message: string
    ) {
        super(message);
    }
}

let hostHasNoHandler = false;

/** The lane, when this window has it and the host has not said it has no handler. */
function lane(): NonNullable<typeof window.agentmuxQuery> | null {
    if (hostHasNoHandler) return null;
    const fn = window.agentmuxQuery;
    return typeof fn === "function" ? fn : null;
}

/** Forget that the host refused the lane (tests). */
export function resetQueryLane(): void {
    hostHasNoHandler = false;
}

/** Whether the next command would go over the lane. */
export function queryLaneAvailable(): boolean {
    return lane() !== null && !!window.__AGENTMUX_IPC_TOKEN__;
}

/**
 * Run a host command over the lane when this window has one, else over HTTP
 * like `invokeCommand`. The host checks the IPC token on both.
 */
export async function invokeFastCommand<T = unknown>(cmd: string, args: Record<string, unknown>): Promise<T> {
    const query = lane();
    const token = window.__AGENTMUX_IPC_TOKEN__;
    if (!query || !token) return invokeCommand<T>(cmd, args);
    const t0 = performance.now();
    try {
        const response = await new Promise<string>((resolve, reject) => {
            query({
                request: JSON.stringify({ cmd, token, args }),
                persistent: false,
                onSuccess: resolve,
                onFailure: (code, message) => reject(new QueryFailure(code, message)),
            });
        });
        recordIpcRoundtrip(cmd, performance.now() - t0);
        return JSON.parse(response) as T;
    } catch (e) {
        if (e instanceof QueryFailure && e.code === NO_HANDLER) hostHasNoHandler = true;
        return invokeCommand<T>(cmd, args);
    }
}
