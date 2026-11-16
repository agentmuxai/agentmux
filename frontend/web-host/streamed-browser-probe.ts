// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Does the server this tab is signed in to offer streamed Browser panes? The route answers 404 when the server was
// not started with them and "upgrade required" to a plain request when it was, so one request tells. A server that
// cannot be asked, or does not answer in time, is taken to have none: the pane then shows its usual notice.

import { SESSION_HEADER } from "@/util/sharedconst";

const ROUTE = "/agentmux/streamed-browser";
/** The app's own start waits only seconds for the host API: the server is not asked for longer. */
const PROBE_LIMIT_MS = 2000;

export async function streamedBrowserAvailable(
    token: string | undefined,
    fetchImpl: (input: string, init: RequestInit) => Promise<{ status: number }> = (input, init) => fetch(input, init)
): Promise<boolean> {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), PROBE_LIMIT_MS);
    try {
        const headers: Record<string, string> = token ? { [SESSION_HEADER]: token } : {};
        const resp = await fetchImpl(ROUTE, { headers, credentials: "same-origin", signal: ctl.signal });
        return resp.status === 426;
    } catch {
        return false;
    } finally {
        clearTimeout(timer);
    }
}
