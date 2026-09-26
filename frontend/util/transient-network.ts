// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * True for a request that never got a response — Chromium rejects `fetch`
 * with `TypeError: Failed to fetch` (Firefox: "NetworkError…"). On Windows a
 * loopback connect can be refused this way (WSAENOBUFS, 10055) while srv is
 * perfectly healthy, so a retry usually succeeds. A backend error response,
 * or any other TypeError (a code bug), is NOT transient. See #3868.
 */
export function isTransientNetworkError(err: unknown): boolean {
    return err instanceof TypeError && /failed to fetch|networkerror|load failed/i.test(err.message);
}

/** ~1.1 s in total: 10055 refusals seen on #3868's machine came ~1 s apart. */
export const TRANSIENT_RETRY_DELAYS_MS: readonly number[] = [100, 300, 700];

/**
 * Run `fn`, retrying after each delay while it fails with a transient network
 * error. Only for idempotent requests: a "Failed to fetch" can also mean the
 * request reached the server and the response was lost.
 */
export async function retryTransient<T>(
    fn: () => Promise<T>,
    delaysMs: readonly number[] = TRANSIENT_RETRY_DELAYS_MS
): Promise<T> {
    for (let attempt = 0; ; attempt++) {
        try {
            return await fn();
        } catch (err) {
            if (attempt >= delaysMs.length || !isTransientNetworkError(err)) throw err;
            await new Promise((resolve) => setTimeout(resolve, delaysMs[attempt]));
        }
    }
}
