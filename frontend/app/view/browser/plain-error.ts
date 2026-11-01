// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A failure of a call from the browser view to AgentMux, in words for the person.
 *
 * The host already says "couldn't reach AgentMux" for the calls it makes to srv
 * (`client/srv_failure.rs`); this is the net under the rest: anything that still
 * carries the operating system's or the network stack's own words (a banner used to
 * show "read from srv: A connection attempt failed … (os error 10060)").
 */

const RAW =
    /os error \d+|connection attempt failed|connected party did not properly respond|\b(read|write|connect)(ing)? (from|to) srv\b|ECONN\w+|ETIMEDOUT|ENOTFOUND|EPIPE|failed to fetch|networkerror|network request failed|socket hang up/i;

/** Whether `message` is the system's own wording for a connection that failed. */
export function isRawNetworkFailure(message: string): boolean {
    return RAW.test(message);
}

/** The plain sentence for a call that could not reach AgentMux. `what` completes
 *  "Couldn't reach AgentMux to …" (e.g. "turn on auto-approve"). */
export function unreachableText(what: string): string {
    return `Couldn't reach AgentMux to ${what}. Nothing was changed. Try again in a moment.`;
}

/**
 * `err` for the person. A failure to reach AgentMux becomes `unreachableText(what)`;
 * any other message (srv's own reason, already worded for the person) is shown as it is,
 * through `wrap` when the caller puts a lead-in before it.
 */
export function plainError(err: unknown, what: string, wrap: (message: string) => string = (m) => m): string {
    const message = String((err as { message?: unknown } | null | undefined)?.message ?? err ?? "").trim();
    if (!message) return `Couldn't ${what}.`;
    return isRawNetworkFailure(message) ? unreachableText(what) : wrap(message);
}
