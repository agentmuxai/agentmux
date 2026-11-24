// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a person can do about a stop that is coming, from AgentMux Web: keep the workspace running, or stop it now. Each is one
// request to the page's own server (the workspace's address), carrying the page's sign-in cookie and nothing the page chose: which
// workspace is the address's, not ours. A server with no such route (the desktop app's own) answers 404 or 405, which reads as an
// error here and is never shown, because the chip only shows when a stop is pending, which only a hosted workspace has.

import { t } from "@/app/i18n/i18n";

/** `ok`, or `message`: what to tell the person. */
export type StopActionResult = { ok: boolean; message?: string };

type Fetch = (input: string, init: RequestInit) => Promise<{ status: number; json(): Promise<unknown> }>;

/** What the server said in words, when it said any (it never says anything a person should not read). */
async function messageOf(resp: { json(): Promise<unknown> }): Promise<string> {
    try {
        const body = (await resp.json()) as { message?: unknown } | null;
        return typeof body?.message === "string" ? body.message : "";
    } catch {
        return "";
    }
}

async function post(route: string, body: unknown, fetchImpl: Fetch): Promise<StopActionResult> {
    let resp;
    try {
        resp = await fetchImpl(route, { method: "POST", credentials: "same-origin", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    } catch {
        return { ok: false, message: t("workspaceStop.unreachable") };
    }
    if (resp.status === 200) return { ok: true };
    if (resp.status === 403) return { ok: false, message: t("workspaceStop.signedOut") };
    return { ok: false, message: (await messageOf(resp)) || t("workspaceStop.failed") };
}

const realFetch: Fetch = (input, init) => fetch(input, init);

/** Cancels the stop that is coming. The workspace announces that it is cleared, and the chip goes. */
export function keepRunning(fetchImpl: Fetch = realFetch): Promise<StopActionResult> {
    return post("/me/keep-running", {}, fetchImpl);
}

/** Ends the wait: the workspace stops now and saves its data. */
export function stopNow(fetchImpl: Fetch = realFetch): Promise<StopActionResult> {
    return post("/me/stop", { mode: "now" }, fetchImpl);
}
