// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Before a streamed pane opens its socket, a server that runs the pane's browser for each person is asked for it: one request starts
// the browser and the following ones, a couple of seconds apart, say when it is up. The answer's cookie (set by the server, never
// read here) is what lets the socket through, and it opens one connection, so every socket the pane opens asks first. A server
// with no such route (the desktop app's own, a local server) answers 404 or 405, and the pane goes straight to its socket as before.
// When the pane goes, the browser is released.

import { t } from "@/app/i18n/i18n";

const ROUTE = "/me/browser";
/** How often a browser that is starting is asked about. */
const POLL_MS = 2000;
/** The server gives up on a start by itself; this is only a bound for a page that is never told. */
const GIVE_UP_MS = 6 * 60_000;

/** `ok`: go on to the socket. Otherwise `message` is what to tell the person (empty when the pane itself went away: `aborted`). */
export type LeaseOutcome = { ok: boolean; message?: string; aborted?: boolean };

type Fetch = (input: string, init: RequestInit) => Promise<{ status: number; json(): Promise<unknown> }>;

export interface LeaseOptions {
    signal: AbortSignal;
    /** The browser is not up yet: called for each answer that says so. */
    onStarting: () => void;
    fetchImpl?: Fetch;
    /** Waits `ms`, or less when `signal` aborts. */
    sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
    now?: () => number;
}

/** Whether a server answered this page's asks for a browser: only then is one released when the pane goes. */
let leased = false;

function wait(ms: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve) => {
        if (signal.aborted) return resolve();
        const timer = setTimeout(done, ms);
        function done() {
            clearTimeout(timer);
            signal.removeEventListener("abort", done);
            resolve();
        }
        signal.addEventListener("abort", done);
    });
}

async function bodyOf(resp: { json(): Promise<unknown> }): Promise<{ ready?: boolean; error?: string; message?: string } | null> {
    try {
        const body = await resp.json();
        return body && typeof body === "object" ? (body as { ready?: boolean; error?: string; message?: string }) : null;
    } catch {
        return null;
    }
}

/**
 * Asks the server for this pane's browser until it is ready: `{ ok: true }` to go on to the socket (also when the server has no
 * such route), or `{ ok: false, message }` with what the server said. Never throws; an aborted wait answers `aborted`.
 */
export async function ensureLease(o: LeaseOptions): Promise<LeaseOutcome> {
    const fetchImpl: Fetch = o.fetchImpl ?? ((input, init) => fetch(input, init));
    const sleep = o.sleep ?? wait;
    const now = o.now ?? Date.now;
    const until = now() + GIVE_UP_MS;
    const aborted = (): LeaseOutcome => ({ ok: false, message: "", aborted: true });
    for (;;) {
        let resp;
        try {
            resp = await fetchImpl(ROUTE, { method: "POST", credentials: "same-origin", headers: { "content-type": "application/json" }, body: "{}", signal: o.signal });
        } catch {
            // Aborted, or a server that cannot be reached: the socket is the next thing to try (and says so if it cannot be opened).
            // An ask that was cut off may still have been answered by a server that began a browser for it: that one is given back too.
            if (o.signal.aborted) {
                leased = true;
                return aborted();
            }
            return { ok: true };
        }
        if (o.signal.aborted) {
            leased = true;
            return aborted();
        }
        const body = await bodyOf(resp);
        // No such route here: a server that does not hand out browsers (a page's own fallback answers 200 with something that is not
        // JSON). A 404 that says "not-found" is the route answering, about a workspace.
        if (body === null || resp.status === 405 || (resp.status === 404 && body.error !== "not-found")) return { ok: true };
        leased = true;
        if (resp.status === 200 && body.ready === true) return { ok: true };
        if (resp.status !== 200) return { ok: false, message: body.message || t("browser.streamed.leaseFailed") };
        if (now() >= until) return { ok: false, message: t("browser.streamed.leaseTimeout") };
        o.onStarting();
        await sleep(POLL_MS, o.signal);
        if (o.signal.aborted) return aborted();
    }
}

/** Gives the page's browser back: best effort, and only if a server handed one out (or may have, to an ask that was cut off). */
export function releaseLease(fetchImpl: Fetch = (input, init) => fetch(input, init)): void {
    if (!leased) return;
    leased = false;
    void fetchImpl(ROUTE, { method: "DELETE", credentials: "same-origin", keepalive: true }).catch(() => {});
}

/** The panes of this page that may be using the browser. The server's browser is the person's, one for the page's panes. */
let panes = 0;

/**
 * A pane takes its place among the page's panes; the function it returns gives the place up when the pane goes (not when it is only
 * hidden). The browser is given back when the LAST pane goes: a pane closing must not take the browser from one that is still open.
 */
export function joinLease(fetchImpl?: Fetch): () => void {
    panes += 1;
    let left = false;
    return () => {
        if (left) return;
        left = true;
        panes = Math.max(0, panes - 1);
        if (panes === 0) releaseLease(fetchImpl);
    };
}

let pageHideWired = false;
/** Gives the browser back when the page itself goes (closing the window is not a pane's cleanup). Wired once. */
export function releaseOnPageHide(target: Pick<Window, "addEventListener"> = window): void {
    if (pageHideWired) return;
    pageHideWired = true;
    target.addEventListener("pagehide", () => releaseLease());
}

/** For tests: forget what a server has answered. */
export function resetLeaseState(): void {
    leased = false;
    panes = 0;
    pageHideWired = false;
}
